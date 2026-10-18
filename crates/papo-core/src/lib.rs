//! Sans-I/O IRC session state and protocol behavior.
//! Time and randomness enter through explicit inputs.

#![cfg_attr(docsrs, feature(doc_cfg))]

mod account_registration;
mod batch;
#[cfg(feature = "unstable")]
mod bouncer;
pub mod cap;
mod caps;
mod channel_rename;
mod features;
mod framing;
mod history;
mod isupport;
mod keepalive;
mod metadata;
mod monitor;
mod outgoing;
#[cfg(feature = "unstable")]
mod persistence;
mod ratelimit;
mod refusal;
mod registration;
#[cfg(feature = "unstable")]
mod relaymsg;
mod sasl;
mod secret;
mod state;
pub mod sts;
#[cfg(feature = "unstable")]
mod webpush;

pub use secret::Secret;

use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};
use web_time::Instant;

use papo_proto::{
    BuildError, Casemapping, Command, Ctcp, MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message,
    MessageRef, ModeChange, ModeKind, Numeric, Source, StandardReply as StandardReplyView,
    StandardReplyKind,
};

use account_registration::{AccountRegistration, PendingRegistration, RegistrationPolicy};
use batch::{BatchAction, BatchEvent, Batches, DispatchContext};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use bouncer::{BouncerNetwork, NetworkConfig, NetworkId, NetworkState};
use caps::Caps;
pub use features::{Feature, Unsupported, UnsupportedReason};
use framing::Framer;
pub use history::{HistoryQuery, LastSeen, MsgRef, ReadMarker};
use isupport::IsupportState;
use keepalive::Keepalive;
pub use metadata::{MetadataCommand, MetadataLimits, MetadataValue};
use monitor::Monitor;
pub use outgoing::TextKind;
use ratelimit::RateLimiter;
pub use ratelimit::{InvalidRateLimit, RateLimit};
pub use refusal::Refusal;
use registration::Registration;
use sasl::SaslSession;
pub use sasl::{Entropy, Sasl};
pub use state::{Account, Away, Channel, Member, State, User};
use state::{MetadataContext, StateSnapshot};
use sts::ConnectionSecurity;

const MAX_INCOMING_LINE_LEN: usize = MAX_TAG_SECTION_LEN + MAX_MESSAGE_LEN;
const DEFAULT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_PING_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MAX_CHANNELS: usize = 10_000;
const DEFAULT_MAX_MEMBERS_PER_CHANNEL: usize = 100_000;
const DEFAULT_MAX_OPEN_BATCHES: usize = 64;
const DEFAULT_MAX_BATCH_LINES: usize = 10_000;
const DEFAULT_BATCH_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PENDING_REQUESTS: usize = 256;
const MAX_CTCP_SOURCES: usize = 1024;

/// Details about text queued by [`Session::send_text_detailed`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SendTextResult {
    /// Correlation identifier when labeled responses are enabled.
    pub request: Option<RequestId>,
    /// Whether the text was divided into multiple message lines.
    pub split: bool,
}
const CTCP_REPLY_INTERVAL: Duration = Duration::from_secs(2);

/// Capabilities requested by a default session when the server advertises them.
pub const DEFAULT_WANTED_CAPS: &[&str] = &[
    "cap-notify",
    "message-tags",
    "server-time",
    "batch",
    "draft/event-playback",
    "echo-message",
    "account-tag",
    "extended-join",
    "multi-prefix",
    "userhost-in-names",
    "chghost",
    "setname",
    "away-notify",
    "account-notify",
    "extended-monitor",
    "invite-notify",
    "standard-replies",
    "labeled-response",
    "draft/multiline",
    "draft/oper-tag",
    "draft/metadata-2",
    "draft/account-registration",
    #[cfg(feature = "unstable")]
    "soju.im/bouncer-networks",
    #[cfg(feature = "unstable")]
    "soju.im/bouncer-networks-notify",
    "chathistory",
    "draft/read-marker",
    #[cfg(feature = "unstable")]
    "draft/persistence",
    "draft/channel-rename",
    #[cfg(feature = "unstable")]
    "draft/relaymsg",
    "draft/message-redaction",
    #[cfg(feature = "unstable")]
    "draft/webpush",
    "draft/pre-away",
    "draft/no-implicit-names",
    "draft/whoami",
    "draft/extended-isupport",
];

/// Enabled `IRCv3` capability names and their advertised values.
pub type Capabilities = BTreeMap<String, Option<String>>;

/// Automatic replies to private CTCP queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CtcpReplies {
    /// VERSION reply, or `None` to disable it.
    pub version: Option<String>,
    /// Whether PING parameters are echoed.
    pub ping: bool,
    /// TIME reply supplied by a shell with access to a wall clock.
    pub time: Option<String>,
    /// SOURCE reply, or `None` to disable it.
    pub source: Option<String>,
}

/// Away state to announce when connecting.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AwayOnConnect {
    /// Announce this message. Without `draft/pre-away`, it is sent after registration.
    Message(String),
    /// Announce hidden away state with `AWAY *`. Without `draft/pre-away`, it is skipped.
    Hidden,
}

impl Default for CtcpReplies {
    fn default() -> Self {
        Self {
            version: Some(format!("papo {}", env!("CARGO_PKG_VERSION"))),
            ping: true,
            time: None,
            source: None,
        }
    }
}

/// Registration state captured when a session becomes ready for use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Welcome {
    /// The nickname assigned by the server.
    pub nick: String,
    /// The human-readable text from the server's `001` welcome numeric.
    pub text: String,
    /// The enabled capabilities and their advertised values.
    pub caps: Capabilities,
    /// The accumulated server feature advertisement.
    pub isupport: papo_proto::Isupport,
}

/// Policy for decoding incoming IRC lines.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Decoding {
    /// Require incoming lines to contain valid UTF-8.
    Utf8,
    /// Decode UTF-8 when possible and otherwise map each byte as Latin-1.
    #[default]
    Utf8OrLatin1,
}

/// Host and transport security for one connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Connection {
    /// Hostname used to establish the connection and identify its STS policy.
    pub host: String,
    /// Security of the connected transport.
    pub security: ConnectionSecurity,
}

/// Identity asserted by a trusted IRC gateway with `WEBIRC`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebIrc {
    /// Password shared with the IRC server.
    pub password: Secret,
    /// Gateway name configured on the IRC server.
    pub gateway: String,
    /// Client hostname, or `None` when only its IP address is known.
    pub hostname: Option<String>,
    /// Client IP address.
    pub ip: String,
    /// Ordered `name[=value]` options appended to the command.
    pub options: Vec<(String, Option<String>)>,
}

/// Configuration for an IRC session.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct Config {
    /// The policy used to decode incoming lines.
    pub decoding: Decoding,
    /// The nickname requested during registration.
    pub nick: String,
    /// The username sent during registration.
    pub username: String,
    /// The real name sent during registration.
    pub realname: String,
    /// The optional server password sent during registration.
    pub password: Option<Secret>,
    /// Gateway identity sent before registration begins.
    pub webirc: Option<WebIrc>,
    /// The maximum time to wait for the welcome numeric.
    pub registration_timeout: Duration,
    /// Idle time after which the client probes the server with `PING`.
    pub ping_interval: Duration,
    /// Maximum silence before the session closes the connection.
    pub ping_timeout: Duration,
    /// Outbound line rate limit, ignored when the server advertises `SAFERATE` unless strict.
    pub rate_limit: RateLimit,
    /// Default transport security used by [`Session::handle_connected`].
    pub connection_security: ConnectionSecurity,
    /// Default hostname used by [`Session::handle_connected`].
    pub host: String,
    /// Capability names to request when advertised by the server.
    pub wanted_caps: Vec<String>,
    /// Capability names explicitly disabled by the client.
    pub disabled_caps: Vec<String>,
    /// Alternative nicknames to try when registration collides.
    pub alt_nicks: Vec<String>,
    /// SASL mechanisms to try in preference order.
    pub sasl: Option<Sasl>,
    /// Ergo multiclient device identifier appended to the authentication identity.
    pub device: Option<String>,
    /// Maximum number of channels retained in session state.
    pub max_channels: usize,
    /// Maximum number of members retained for one channel.
    pub max_members_per_channel: usize,
    /// Maximum number of batches retained at once.
    pub max_open_batches: usize,
    /// Maximum number of server lines retained in one batch.
    pub max_batch_lines: usize,
    /// Maximum time an incomplete batch remains open.
    pub batch_timeout: Duration,
    /// Maximum time a labelled request remains pending.
    pub request_timeout: Duration,
    /// Automatic CTCP reply policy.
    pub ctcp: CtcpReplies,
    /// Away state announced during or immediately after registration.
    pub away_on_connect: Option<AwayOnConnect>,
    /// Metadata keys subscribed on every connection.
    pub metadata_subscriptions: Vec<String>,
    /// Upstream bouncer network selected before registration completes.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub bouncer_network: Option<NetworkId>,
    registration_on_connect: Option<PendingRegistration>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("Config");
        debug
            .field("decoding", &self.decoding)
            .field("nick", &self.nick)
            .field("username", &self.username)
            .field("realname", &self.realname)
            .field("password", &self.password.as_ref().map(|_| RedactedSecret))
            .field("webirc", &self.webirc)
            .field("registration_timeout", &self.registration_timeout)
            .field("ping_interval", &self.ping_interval)
            .field("ping_timeout", &self.ping_timeout)
            .field("rate_limit", &self.rate_limit)
            .field("connection_security", &self.connection_security)
            .field("host", &self.host)
            .field("wanted_caps", &self.wanted_caps)
            .field("disabled_caps", &self.disabled_caps)
            .field("alt_nicks", &self.alt_nicks)
            .field("sasl", &self.sasl)
            .field("device", &self.device)
            .field("max_channels", &self.max_channels)
            .field("max_members_per_channel", &self.max_members_per_channel)
            .field("max_open_batches", &self.max_open_batches)
            .field("max_batch_lines", &self.max_batch_lines)
            .field("batch_timeout", &self.batch_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("ctcp", &self.ctcp)
            .field("away_on_connect", &self.away_on_connect)
            .field("metadata_subscriptions", &self.metadata_subscriptions);
        #[cfg(feature = "unstable")]
        debug.field("bouncer_network", &self.bouncer_network);
        debug
            .field("registration_on_connect", &self.registration_on_connect)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct RedactedSecret;

impl std::fmt::Debug for RedactedSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            decoding: Decoding::default(),
            nick: "papo".to_owned(),
            username: "papo".to_owned(),
            realname: "papo".to_owned(),
            password: None,
            webirc: None,
            registration_timeout: DEFAULT_REGISTRATION_TIMEOUT,
            ping_interval: DEFAULT_PING_INTERVAL,
            ping_timeout: DEFAULT_PING_TIMEOUT,
            rate_limit: RateLimit::default(),
            connection_security: ConnectionSecurity::Plaintext,
            host: String::new(),
            wanted_caps: DEFAULT_WANTED_CAPS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            disabled_caps: Vec::new(),
            alt_nicks: Vec::new(),
            sasl: None,
            device: None,
            max_channels: DEFAULT_MAX_CHANNELS,
            max_members_per_channel: DEFAULT_MAX_MEMBERS_PER_CHANNEL,
            max_open_batches: DEFAULT_MAX_OPEN_BATCHES,
            max_batch_lines: DEFAULT_MAX_BATCH_LINES,
            batch_timeout: DEFAULT_BATCH_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            ctcp: CtcpReplies::default(),
            away_on_connect: None,
            metadata_subscriptions: Vec::new(),
            #[cfg(feature = "unstable")]
            bouncer_network: None,
            registration_on_connect: None,
        }
    }
}

impl Config {
    /// Sets the incoming line decoding policy.
    #[must_use]
    pub const fn decoding(mut self, decoding: Decoding) -> Self {
        self.decoding = decoding;
        self
    }

    /// Sets the nickname requested during registration.
    #[must_use]
    pub fn nick(mut self, nick: impl Into<String>) -> Self {
        self.nick = nick.into();
        self
    }

    /// Sets the username sent during registration.
    #[must_use]
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = username.into();
        self
    }

    /// Sets the real name sent during registration.
    #[must_use]
    pub fn realname(mut self, realname: impl Into<String>) -> Self {
        self.realname = realname.into();
        self
    }

    /// Sets the optional server password sent during registration.
    #[must_use]
    pub fn password(mut self, password: impl Into<Secret>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the gateway identity sent with `WEBIRC` before registration.
    #[must_use]
    pub fn webirc(mut self, webirc: WebIrc) -> Self {
        self.webirc = Some(webirc);
        self
    }

    /// Stages account registration for servers supporting the before-connect extension.
    #[must_use]
    pub fn register_on_connect(
        mut self,
        account: Option<impl Into<String>>,
        email: Option<impl Into<String>>,
        password: impl Into<Secret>,
    ) -> Self {
        self.registration_on_connect = Some(PendingRegistration {
            account: account.map(Into::into),
            email: email.map(Into::into),
            password: password.into(),
        });
        self
    }

    /// Sets the maximum time to wait for the welcome numeric.
    #[must_use]
    pub const fn registration_timeout(mut self, timeout: Duration) -> Self {
        self.registration_timeout = timeout;
        self
    }

    /// Sets the idle interval before sending a keepalive probe.
    #[must_use]
    pub const fn ping_interval(mut self, interval: Duration) -> Self {
        self.ping_interval = interval;
        self
    }

    /// Sets the maximum time allowed without an incoming line.
    #[must_use]
    pub const fn ping_timeout(mut self, timeout: Duration) -> Self {
        self.ping_timeout = timeout;
        self
    }

    /// Sets the outbound line rate limit.
    ///
    /// The limit is ignored while the server advertises `SAFERATE` unless
    /// [`RateLimit::strict`] was selected.
    #[must_use]
    pub const fn rate_limit(mut self, rate_limit: RateLimit) -> Self {
        self.rate_limit = rate_limit;
        self
    }

    /// Sets the current transport security.
    #[must_use]
    pub const fn connection_security(mut self, security: ConnectionSecurity) -> Self {
        self.connection_security = security;
        self
    }

    /// Sets the hostname used for the current connection.
    #[must_use]
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sets the capability names to request when advertised by the server.
    #[must_use]
    pub fn wanted_caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.wanted_caps = caps.into_iter().map(Into::into).collect();
        self
    }

    /// Prevents the named capabilities from being requested.
    #[must_use]
    pub fn without_caps<I, S>(mut self, caps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for cap in caps.into_iter().map(Into::into) {
            if !self.cap_disabled(&cap) {
                self.disabled_caps.push(cap);
            }
        }
        self
    }

    fn cap_disabled(&self, name: &str) -> bool {
        self.disabled_caps
            .iter()
            .any(|disabled| papo_proto::canonical(disabled) == papo_proto::canonical(name))
    }

    /// Sets the alternative nicknames tried during registration.
    #[must_use]
    pub fn alt_nicks<I, S>(mut self, nicks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.alt_nicks = nicks.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the SASL mechanisms to try during registration.
    #[must_use]
    pub fn sasl(mut self, sasl: Sasl) -> Self {
        self.sasl = Some(sasl);
        self
    }

    /// Sets the Ergo multiclient device identifier.
    #[must_use]
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }

    /// Sets the maximum number of channels retained in session state.
    #[must_use]
    pub const fn max_channels(mut self, maximum: usize) -> Self {
        self.max_channels = maximum;
        self
    }

    /// Sets the maximum members retained for each channel in session state.
    #[must_use]
    pub const fn max_members_per_channel(mut self, maximum: usize) -> Self {
        self.max_members_per_channel = maximum;
        self
    }

    /// Sets the maximum number of batches retained at once.
    #[must_use]
    pub const fn max_open_batches(mut self, maximum: usize) -> Self {
        self.max_open_batches = maximum;
        self
    }

    /// Sets the maximum number of server lines retained in one batch.
    #[must_use]
    pub const fn max_batch_lines(mut self, maximum: usize) -> Self {
        self.max_batch_lines = maximum;
        self
    }

    /// Sets how long an incomplete batch remains open.
    #[must_use]
    pub const fn batch_timeout(mut self, timeout: Duration) -> Self {
        self.batch_timeout = timeout;
        self
    }

    /// Sets how long a labelled request remains pending.
    #[must_use]
    pub const fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Sets automatic CTCP replies. Channel queries receive a private reply.
    #[must_use]
    pub fn ctcp(mut self, replies: CtcpReplies) -> Self {
        self.ctcp = replies;
        self
    }

    /// Sets away state to announce while connecting.
    ///
    /// With `draft/pre-away`, both variants are sent before `CAP END`. Without it, a message is
    /// sent after registration completes and hidden away state is skipped.
    #[must_use]
    pub fn away_on_connect(mut self, away: Option<AwayOnConnect>) -> Self {
        self.away_on_connect = away;
        self
    }

    /// Sets metadata keys subscribed on every connection.
    #[must_use]
    pub fn metadata_subscriptions<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.metadata_subscriptions = keys.into_iter().map(Into::into).collect();
        self
    }
}

/// An owned operation from an IRC mode string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedModeChange {
    /// Whether the mode is being set rather than removed.
    pub set: bool,
    /// The mode letter.
    pub mode: char,
    /// The mode argument, when one was required and supplied.
    pub arg: Option<String>,
    /// The mode's advertised parameter behavior.
    pub kind: ModeKind,
}

/// Stable identifier assigned to one user-originated request.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestId(u64);

/// Response boundary used when a server does not support `labeled-response`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResponseMatcher {
    /// A `WHOIS` reply ending at `318` or an error naming this nickname.
    Whois(String),
    /// A `WHO` reply ending at `315` for this mask.
    Who(String),
    /// A channel join ending when initial channel state is synchronized.
    Join(String),
    /// A channel departure ending at our `PART` echo.
    Part(String),
    /// A mode change ending at its `MODE` echo.
    Mode(String),
    /// A topic request ending at its echo or topic numeric.
    Topic(String),
    /// A channel removal ending at its `KICK` echo.
    Kick {
        /// Channel being changed.
        channel: String,
        /// Nickname being removed.
        nick: String,
    },
    /// A channel invitation ending at `RPL_INVITING`.
    Invite {
        /// Channel named by the invitation.
        channel: String,
        /// Nickname being invited.
        nick: String,
    },
    /// A nickname change ending at our `NICK` echo.
    Nick(String),
    /// An away-state change ending at `RPL_NOWAWAY` or `RPL_UNAWAY`.
    Away(bool),
    /// A realname change ending at our `SETNAME` echo.
    SetName(String),
    /// An operator login ending at `RPL_YOUREOPER`.
    Oper,
    /// A latency probe ending at the matching `PONG`.
    Ping(String),
    /// A channel list ending at `323`.
    List,
}

impl ResponseMatcher {
    fn standard_failure(message: &MessageRef<'_>, command: &str) -> bool {
        StandardReplyView::parse(message).is_some_and(|reply| {
            reply.kind == StandardReplyKind::Fail && reply.command.eq_ignore_ascii_case(command)
        })
    }

    fn target_is(
        message: &MessageRef<'_>,
        index: usize,
        target: &str,
        mapping: Casemapping,
    ) -> bool {
        message
            .param(index)
            .is_some_and(|value| mapping.eq(value, target))
    }

    fn numeric_names_command(message: &MessageRef<'_>, numeric: Numeric, command: &str) -> bool {
        numeric.is_error()
            && message
                .param(1)
                .is_some_and(|value| value.eq_ignore_ascii_case(command))
    }

    fn own_channel_departure(
        message: &MessageRef<'_>,
        channel: &str,
        mapping: Casemapping,
        own_nick: &str,
        own_source: bool,
    ) -> bool {
        (message.command() == Command::Part
            && own_source
            && Self::target_is(message, 0, channel, mapping))
            || (message.command() == Command::Kick
                && Self::target_is(message, 0, channel, mapping)
                && message
                    .param(1)
                    .is_some_and(|nick| mapping.eq(nick, own_nick)))
    }

    #[allow(clippy::too_many_lines)]
    fn matches(
        &self,
        message: &MessageRef<'_>,
        mapping: Casemapping,
        own_nick: &str,
        state: &State,
    ) -> Option<bool> {
        let numeric = match message.command() {
            Command::Numeric(numeric) => Some(numeric),
            _ => None,
        };
        let own_source = matches!(message.source(), Some(Source::User { nick, .. }) if mapping.eq(nick, own_nick));
        let current_own_source = matches!(message.source(), Some(Source::User { nick, .. }) if mapping.eq(nick, state.nick()));
        match self {
            Self::Whois(nick) => {
                if Self::standard_failure(message, "WHOIS") {
                    return Some(true);
                }
                let Command::Numeric(numeric) = message.command() else {
                    return None;
                };
                let relevant = matches!(
                    numeric,
                    Numeric::RPL_AWAY
                        | Numeric::RPL_WHOISUSER
                        | Numeric::RPL_WHOISSERVER
                        | Numeric::RPL_WHOISIDLE
                        | Numeric::RPL_ENDOFWHOIS
                        | Numeric::RPL_WHOISCHANNELS
                        | Numeric::RPL_WHOISACCOUNT
                        | Numeric::RPL_WHOISBOT
                        | Numeric::RPL_WHOISSECURE
                ) || numeric.is_error();
                (relevant && Self::target_is(message, 1, nick, mapping))
                    .then_some(numeric == Numeric::RPL_ENDOFWHOIS || numeric.is_error())
            }
            Self::Who(mask) => {
                if Self::standard_failure(message, "WHO") {
                    return Some(true);
                }
                numeric.and_then(|numeric| {
                    let row_matches = match numeric {
                        Numeric::RPL_WHOREPLY => {
                            Self::target_is(message, 1, mask, mapping)
                                || Self::target_is(message, 5, mask, mapping)
                        }
                        Numeric::RPL_WHOSPCRPL => {
                            Self::target_is(message, 2, mask, mapping)
                                || Self::target_is(message, 5, mask, mapping)
                        }
                        _ => false,
                    };
                    (row_matches || Self::target_is(message, 1, mask, mapping))
                        .then_some(numeric == Numeric::RPL_ENDOFWHO || numeric.is_error())
                })
            }
            Self::Join(channel) => {
                if Self::standard_failure(message, "JOIN") {
                    return Some(true);
                }
                let named = |index| Self::target_is(message, index, channel, mapping);
                if message.command() == Command::Join && own_source && named(0) {
                    return Some(state.channel(channel).is_some_and(Channel::is_synced));
                }
                if Self::own_channel_departure(message, channel, mapping, own_nick, own_source) {
                    return Some(true);
                }
                numeric.and_then(|numeric| {
                    (named(1) && (numeric == Numeric::RPL_ENDOFNAMES || numeric.is_error()))
                        .then_some(true)
                })
            }
            Self::Part(channel) => ((message.command() == Command::Part
                && own_source
                && Self::target_is(message, 0, channel, mapping))
                || (numeric.is_some_and(Numeric::is_error)
                    && Self::target_is(message, 1, channel, mapping))
                || Self::standard_failure(message, "PART"))
            .then_some(true),
            Self::Mode(target) => ((message.command() == Command::Mode
                && Self::target_is(message, 0, target, mapping))
                || (numeric.is_some_and(Numeric::is_error)
                    && Self::target_is(message, 1, target, mapping))
                || Self::standard_failure(message, "MODE"))
            .then_some(true),
            Self::Topic(channel) => {
                let named = |index| Self::target_is(message, index, channel, mapping);
                ((message.command() == Command::Topic && named(0))
                    || numeric.is_some_and(|numeric| {
                        named(1)
                            && (matches!(numeric, Numeric::RPL_NOTOPIC | Numeric::RPL_TOPIC)
                                || numeric.is_error())
                    })
                    || Self::standard_failure(message, "TOPIC"))
                .then_some(true)
            }
            Self::Kick { channel, nick } => {
                let named = |index, expected| Self::target_is(message, index, expected, mapping);
                ((message.command() == Command::Kick && named(0, channel) && named(1, nick))
                    || numeric.is_some_and(Numeric::is_error)
                        && (named(1, channel) || named(1, nick))
                    || Self::standard_failure(message, "KICK"))
                .then_some(true)
            }
            Self::Invite { channel, nick } => {
                let named = |index, expected| Self::target_is(message, index, expected, mapping);
                (numeric.is_some_and(|numeric| {
                    (numeric == Numeric::RPL_INVITING && named(1, nick) && named(2, channel))
                        || numeric.is_error() && (named(1, channel) || named(1, nick))
                }) || Self::standard_failure(message, "INVITE"))
                .then_some(true)
            }
            Self::Nick(nick) => ((message.command() == Command::Nick
                && own_source
                && Self::target_is(message, 0, nick, mapping))
                || (numeric.is_some_and(Numeric::is_error)
                    && Self::target_is(message, 1, nick, mapping))
                || Self::standard_failure(message, "NICK"))
            .then_some(true),
            Self::Away(away) => (numeric.is_some_and(|numeric| {
                numeric
                    == if *away {
                        Numeric::RPL_NOWAWAY
                    } else {
                        Numeric::RPL_UNAWAY
                    }
                    || Self::numeric_names_command(message, numeric, "AWAY")
            }) || Self::standard_failure(message, "AWAY"))
            .then_some(true),
            Self::SetName(realname) => ((message.command() == Command::Setname
                && current_own_source
                && message.param(0) == Some(realname))
                || numeric.is_some_and(|numeric| {
                    Self::numeric_names_command(message, numeric, "SETNAME")
                })
                || Self::standard_failure(message, "SETNAME"))
            .then_some(true),
            Self::Oper => (numeric.is_some_and(|numeric| {
                matches!(
                    numeric,
                    Numeric::RPL_YOUREOPER | Numeric::ERR_PASSWDMISMATCH | Numeric::ERR_NOOPERHOST
                ) || Self::numeric_names_command(message, numeric, "OPER")
            }) || Self::standard_failure(message, "OPER"))
            .then_some(true),
            Self::Ping(token) => ((message.command() == Command::Pong
                && message.params().any(|param| param == token))
                || Self::standard_failure(message, "PING"))
            .then_some(true),
            Self::List => {
                if Self::standard_failure(message, "LIST") {
                    return Some(true);
                }
                numeric.and_then(|numeric| {
                    let list_error = Self::numeric_names_command(message, numeric, "LIST");
                    (matches!(
                        numeric,
                        Numeric::RPL_LISTSTART | Numeric::RPL_LIST | Numeric::RPL_LISTEND
                    ) || list_error)
                        .then_some(numeric == Numeric::RPL_LISTEND || list_error)
                })
            }
        }
    }
}

#[derive(Debug)]
struct UnlabelledRequest {
    id: RequestId,
    started: Instant,
    matcher: ResponseMatcher,
    fence: Option<String>,
    lines: Vec<Message>,
}

/// One channel reported by a `WHOIS` response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhoisChannel {
    /// Channel name without membership prefixes.
    pub name: String,
    /// Advertised membership prefix symbols in wire order.
    pub prefixes: Vec<char>,
}

/// Completed response to a labelled request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    /// Identifier assigned when the request was queued.
    pub id: RequestId,
    /// Response lines in server delivery order.
    pub lines: Vec<Message>,
    /// A typed history page nested in this response, when present.
    pub history: Option<HistoryPage>,
    /// Targets nested in a completed `CHATHISTORY TARGETS` response.
    pub history_targets: Option<Vec<(String, std::time::SystemTime)>>,
    /// Channels parsed from `RPL_WHOISCHANNELS` using the current `PREFIX` token.
    pub whois_channels: Vec<WhoisChannel>,
    sent: Option<SentInfo>,
}

impl Response {
    /// Returns identifiers assigned to an echoed `PRIVMSG` or `NOTICE`.
    #[must_use]
    pub fn sent(&self) -> Option<SentInfo> {
        self.sent.clone()
    }

    /// Returns the first standard failure or error numeric in this response.
    #[must_use]
    pub fn error(&self) -> Option<ServerError> {
        self.lines.iter().find_map(|message| {
            let message = message.as_ref();
            if let Some(reply) = StandardReplyView::parse(&message)
                && reply.kind == StandardReplyKind::Fail
            {
                return Some(ServerError::Standard {
                    command: reply.command.to_ascii_uppercase(),
                    code: reply.code.to_ascii_uppercase(),
                    context: reply.context.into_iter().map(str::to_owned).collect(),
                    description: reply.description.to_owned(),
                });
            }
            let Command::Numeric(numeric) = message.command() else {
                return None;
            };
            (numeric.is_error() || numeric == Numeric::RPL_TRYAGAIN).then(|| ServerError::Numeric {
                numeric,
                params: message.params().map(str::to_owned).collect(),
            })
        })
    }

    /// Returns the first refusal category in this response.
    #[must_use]
    pub fn refusal(&self) -> Option<Refusal> {
        Some(self.error()?.refusal())
    }
}

/// Server-assigned identity of a message sent by this session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentInfo {
    /// Network-unique message identifier.
    pub msgid: String,
    /// Server time tag, when supplied.
    pub time: Option<String>,
}

/// Error reported by an IRC server while processing a request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ServerError {
    /// An `IRCv3` `FAIL` standard reply.
    Standard {
        /// Command associated with the failure.
        command: String,
        /// Case-normalized failure code.
        code: String,
        /// Additional diagnostic parameters.
        context: Vec<String>,
        /// Human-readable failure description.
        description: String,
    },
    /// A classic error numeric.
    Numeric {
        /// Numeric error code.
        numeric: Numeric,
        /// Numeric parameters in wire order.
        params: Vec<String>,
    },
}

impl ServerError {
    /// Classifies this refusal independently of its wire representation.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        match self {
            Self::Numeric { numeric, .. } => Refusal::from_numeric(*numeric),
            Self::Standard { command, code, .. } => Refusal::from_standard(command, code),
        }
    }
}

/// Owned `IRCv3` standard reply received outside a correlated request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StandardReply {
    /// Reply severity.
    pub kind: StandardReplyKind,
    /// Case-normalized command associated with the reply.
    pub command: String,
    /// Case-normalized reply code.
    pub code: String,
    /// Additional diagnostic parameters.
    pub context: Vec<String>,
    /// Human-readable reply description.
    pub description: String,
}

/// An account's configured persistence preference.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PersistenceSetting {
    /// Keep the account's client active while disconnected.
    On,
    /// Disconnect the account's client normally.
    Off,
    /// Use the server's default persistence policy.
    Default,
}

#[cfg(feature = "unstable")]
impl PersistenceSetting {
    /// Returns the wire token for this setting.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "ON",
            Self::Off => "OFF",
            Self::Default => "DEFAULT",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "ON" => Some(Self::On),
            "OFF" => Some(Self::Off),
            "DEFAULT" => Some(Self::Default),
            _ => None,
        }
    }
}

/// Server-reported account persistence state.
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PersistenceStatus {
    /// The account's configured preference.
    pub setting: PersistenceSetting,
    /// Whether persistence is currently effective.
    pub effective: bool,
}

impl StandardReply {
    /// Returns the refusal category when this is a failure.
    #[must_use]
    pub fn refusal(&self) -> Option<Refusal> {
        (self.kind == StandardReplyKind::Fail)
            .then(|| Refusal::from_standard(&self.command, &self.code))
    }
}

/// One message in a history page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryMessage {
    /// The replayed IRC message.
    pub message: Message,
    /// Whether the server supplied it as context outside the requested limit.
    pub context: bool,
}

impl std::ops::Deref for HistoryMessage {
    type Target = Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

/// One replayed entry in a history page.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HistoryItem {
    /// A `PRIVMSG` or `NOTICE`.
    Message(HistoryMessage),
    /// A reaction or unreaction carried by `TAGMSG`.
    Reacted(HistoryMessage),
    /// A `REDACT` command.
    Redacted(HistoryMessage),
    /// A `TAGMSG` without reaction metadata.
    TagMsg(HistoryMessage),
    /// A `JOIN` command.
    Joined(HistoryMessage),
    /// A `PART` command.
    Parted(HistoryMessage),
    /// A `QUIT` command.
    Quit(HistoryMessage),
    /// A `KICK` command.
    Kicked(HistoryMessage),
    /// A `NICK` command.
    NickChanged(HistoryMessage),
    /// A `TOPIC` command.
    TopicChanged(HistoryMessage),
    /// A `MODE` command.
    ModeChanged(HistoryMessage),
    /// A replayed command without a more specific classification.
    ///
    /// A later minor release may refine a command currently classified here.
    Other(HistoryMessage),
}

impl HistoryItem {
    /// Returns this item as a chat message.
    #[must_use]
    pub const fn message(&self) -> Option<&HistoryMessage> {
        match self {
            Self::Message(message) => Some(message),
            _ => None,
        }
    }

    /// Returns the chat message, or `None` for a replayed activity item.
    #[must_use]
    pub fn into_message(self) -> Option<HistoryMessage> {
        match self {
            Self::Message(message) => Some(message),
            _ => None,
        }
    }

    /// Returns the replayed wire message and its history context flag.
    #[must_use]
    pub fn into_raw(self) -> HistoryMessage {
        match self {
            Self::Message(message)
            | Self::Reacted(message)
            | Self::Redacted(message)
            | Self::TagMsg(message)
            | Self::Joined(message)
            | Self::Parted(message)
            | Self::Quit(message)
            | Self::Kicked(message)
            | Self::NickChanged(message)
            | Self::TopicChanged(message)
            | Self::ModeChanged(message)
            | Self::Other(message) => message,
        }
    }
}

/// History page attached to a completed response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPage {
    /// History target named by the batch.
    pub target: String,
    /// Replayed items in server delivery order.
    pub items: Vec<HistoryItem>,
    /// Whether the server reported that no further page exists.
    ///
    /// A page shorter than the requested limit does not imply the end.
    pub end: bool,
}

impl HistoryPage {
    /// Iterates over chat messages, excluding replayed activity items.
    pub fn messages(&self) -> impl Iterator<Item = &HistoryMessage> {
        self.items.iter().filter_map(HistoryItem::message)
    }
}

/// Compatibility name for a history page attached to a response.
pub type ResponseHistory = HistoryPage;

/// A completed `IRCv3` multiline message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultilineMessage {
    /// The reconstructed `PRIVMSG` or `NOTICE`.
    pub message: Message,
}

/// A received `TAGMSG` and its client-only metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagMessage {
    /// Source prefix without its leading colon.
    pub source: Option<String>,
    /// Message target.
    pub target: String,
    /// Tags in wire order.
    pub tags: Vec<(String, Option<String>)>,
}

/// A server-relayed message redaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Redacted {
    /// Source prefix without its leading colon.
    pub source: Option<String>,
    /// History target containing the message.
    pub target: String,
    /// Identifier of the redacted message.
    pub msgid: String,
    /// Optional human-readable reason.
    pub reason: Option<String>,
}

/// A failure to queue a request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RequestError {
    /// The IRC message could not be represented on the wire.
    Build(BuildError),
    /// The session already has 256 labelled requests awaiting replies.
    PendingLimit,
    /// The text contains no nonempty lines to send.
    EmptyText,
    /// A byte secret was supplied to a text-only protocol field.
    InvalidSecret,
    /// An optional protocol feature is unavailable.
    Unsupported(Unsupported),
    /// The server cannot accept the supplied history reference.
    InvalidMessageReference,
    /// Account registration requires an email address.
    EmailRequired,
    /// The server requires the account name to match the current nickname.
    CustomAccountNameUnsupported,
    /// A metadata key is invalid.
    InvalidMetadataKey {
        /// The rejected key.
        key: String,
        /// Why the key was rejected.
        reason: papo_proto::InvalidIdentifier,
    },
    /// A metadata value exceeds the server's advertised byte limit.
    MetadataValueTooLong {
        /// Advertised maximum.
        maximum: usize,
        /// Supplied byte length.
        actual: usize,
    },
    /// A metadata subscription request exceeds the server's advertised limit.
    TooManyMetadataSubscriptions {
        /// Advertised maximum.
        maximum: usize,
        /// Supplied key count.
        actual: usize,
    },
    /// A relayed nickname contains none of the server-advertised separator characters.
    #[cfg(feature = "unstable")]
    InvalidRelayNickname,
    /// A web push endpoint is not a nonempty HTTPS URL.
    #[cfg(feature = "unstable")]
    InvalidWebPushEndpoint,
    /// A web push key is not valid URL-safe base64.
    #[cfg(feature = "unstable")]
    InvalidWebPushKey,
}

/// Machine-readable reason that a session requested connection closure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CloseKind {
    /// Required SASL authentication failed.
    SaslFailed,
    /// The server rejected the gateway's `WEBIRC` command.
    WebIrcRejected,
    /// IRC registration did not complete before its deadline.
    RegistrationTimeout,
    /// A server line exceeded the protocol framing limit.
    LineTooLong,
    /// The server sent an `ERROR` command.
    ServerError,
    /// The server was silent past the keepalive deadline.
    PingTimeout,
    /// The server requested an STS upgrade to TLS.
    StsUpgrade,
    /// The server requires an authenticated account.
    AccountRequired,
    /// The server rejected the connection password (`464`).
    PasswordIncorrect,
    /// The server banned the connection (`465`).
    Banned,
    /// Another protocol condition requires the connection to close.
    Other,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(error) => error.fmt(formatter),
            Self::PendingLimit => formatter.write_str("too many labelled requests are pending"),
            Self::EmptyText => formatter.write_str("text has no nonempty lines"),
            Self::InvalidSecret => formatter.write_str("byte secret cannot be used as text"),
            Self::Unsupported(unsupported) => unsupported.fmt(formatter),
            Self::InvalidMessageReference => formatter.write_str("unsupported message reference"),
            Self::EmailRequired => formatter.write_str("account registration requires an email"),
            Self::CustomAccountNameUnsupported => {
                formatter.write_str("server does not support a custom account name")
            }
            Self::InvalidMetadataKey { key, .. } => {
                write!(formatter, "invalid metadata key {key:?}")
            }
            Self::MetadataValueTooLong { .. } => formatter.write_str("metadata value is too long"),
            Self::TooManyMetadataSubscriptions { .. } => {
                formatter.write_str("too many metadata subscriptions")
            }
            #[cfg(feature = "unstable")]
            Self::InvalidRelayNickname => {
                formatter.write_str("relayed nickname does not contain an allowed separator")
            }
            #[cfg(feature = "unstable")]
            Self::InvalidWebPushEndpoint => formatter.write_str("invalid web push endpoint"),
            #[cfg(feature = "unstable")]
            Self::InvalidWebPushKey => formatter.write_str("invalid web push key"),
        }
    }
}

impl std::error::Error for RequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Build(error) => Some(error),
            Self::PendingLimit
            | Self::EmptyText
            | Self::InvalidSecret
            | Self::Unsupported(_)
            | Self::InvalidMessageReference
            | Self::EmailRequired
            | Self::CustomAccountNameUnsupported
            | Self::MetadataValueTooLong { .. }
            | Self::TooManyMetadataSubscriptions { .. } => None,
            Self::InvalidMetadataKey { reason, .. } => Some(reason),
            #[cfg(feature = "unstable")]
            Self::InvalidRelayNickname => None,
            #[cfg(feature = "unstable")]
            Self::InvalidWebPushEndpoint | Self::InvalidWebPushKey => None,
        }
    }
}

impl From<BuildError> for RequestError {
    fn from(error: BuildError) -> Self {
        Self::Build(error)
    }
}

impl RequestError {
    fn unsupported(feature: Feature) -> Self {
        Self::Unsupported(Unsupported::new(feature, UnsupportedReason::NotOffered))
    }
}

impl From<ModeChange<'_>> for OwnedModeChange {
    fn from(change: ModeChange<'_>) -> Self {
        Self {
            set: change.set,
            mode: change.mode,
            arg: change.arg.map(str::to_owned),
            kind: change.kind,
        }
    }
}

/// An event emitted by an IRC session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// Connection registration completed.
    Registered(Welcome),
    /// A parsed message not consumed by session protocol behavior.
    Message(ReceivedMessage),
    /// A completed `draft/multiline` batch.
    MultilineMessage(MultilineMessage),
    /// A received tag-only message.
    TagMessage(TagMessage),
    /// A message was redacted from a target's history.
    Redacted(Redacted),
    /// A request was sent without protocol-level response correlation.
    RequestUntracked {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request received its complete logical response.
    Response(Response),
    /// A labelled request did not receive a response before its deadline.
    RequestTimedOut {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request was pending when the connection ended.
    RequestAborted {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A pending request's required feature became unavailable.
    RequestUnsupported {
        /// Identifier assigned to the request.
        id: RequestId,
        /// Feature and current reason for its unavailability.
        unsupported: Unsupported,
    },
    /// An unsolicited `IRCv3` standard reply.
    StandardReply(StandardReply),
    /// Account creation completed and the session is authenticated.
    AccountRegistered {
        /// The registered account name.
        account: String,
        /// Whether the server authenticated the session to the account.
        logged_in: bool,
    },
    /// Account creation requires a verification step.
    VerificationRequired {
        /// The account awaiting verification.
        account: String,
        /// Human-readable instructions from the server.
        message: String,
    },
    /// The server reported the account's persistence state.
    #[cfg(feature = "unstable")]
    PersistenceStatus {
        /// The configured persistence preference.
        setting: PersistenceSetting,
        /// Whether persistence is currently effective.
        effective: bool,
    },
    /// An upstream bouncer network was added, updated, or removed.
    #[cfg(feature = "unstable")]
    BouncerNetwork {
        /// Bouncer-assigned network identifier.
        id: NetworkId,
        /// Complete cached network state, or `None` when removed.
        network: Option<BouncerNetwork>,
    },
    /// A target's cached metadata changed.
    MetadataChanged {
        /// Nickname or channel reported by the server.
        target: String,
        /// Metadata key.
        key: String,
        /// New server-returned value, or `None` when cleared.
        value: Option<String>,
        /// Visibility policy reported by the server.
        visibility: String,
    },
    /// The server deferred metadata synchronization for a target.
    MetadataSyncLater {
        /// Nickname or channel whose synchronization was deferred.
        target: String,
        /// Suggested delay before retrying, when supplied.
        retry_after: Option<Duration>,
    },
    /// A completed history or playback batch.
    History {
        /// The history target named by the batch.
        target: String,
        /// Replayed items in server delivery order.
        items: Vec<HistoryItem>,
        /// Whether the server reported that no further page exists.
        end: bool,
    },
    /// Targets that received messages during a requested time range.
    HistoryTargets(Vec<(String, std::time::SystemTime)>),
    /// A target's read marker advanced or was first reported.
    ReadMarker(ReadMarker),
    /// Users departed because connectivity between two servers was lost.
    ///
    /// This event is emitted for an empty batch. Non-`QUIT` lines are delivered as
    /// separate [`Event::Message`] values.
    Netsplit {
        /// The servers on either side of the lost link.
        servers: [String; 2],
        /// The `QUIT` messages grouped by the server.
        quits: Vec<Message>,
    },
    /// Users returned after connectivity between two servers was restored.
    ///
    /// This event is emitted for an empty batch. Non-`JOIN` lines are delivered as
    /// separate [`Event::Message`] values.
    Netjoin {
        /// The servers on either side of the restored link.
        servers: [String; 2],
        /// The `JOIN` messages grouped by the server.
        joins: Vec<Message>,
    },
    /// A server batch exceeded a retention bound or deadline.
    BatchAbandoned {
        /// The opaque batch reference.
        reference: String,
        /// The batch type from its opening line.
        batch_type: String,
        /// The bound that was reached.
        reason: BatchAbandonReason,
    },
    /// A line that could not be decoded or parsed.
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// A request for the transport to drop the connection.
    Close {
        /// Machine-readable closure classification.
        kind: CloseKind,
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// Configured authentication failed and registration is continuing.
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// The capabilities enabled on an established connection changed.
    CapsChanged {
        /// Newly enabled capabilities and updated advertised values.
        added: Capabilities,
        /// Capabilities removed by the server.
        removed: Capabilities,
    },
    /// Optional behaviors available on an established connection changed.
    FeaturesChanged {
        /// Behaviors that became available.
        added: Vec<Feature>,
        /// Behaviors that became unavailable.
        removed: Vec<Feature>,
    },
    /// A plaintext connection must be replaced with TLS on `port`.
    StsUpgrade {
        /// The advertised TLS port.
        port: u16,
    },
    /// An STS persistence action for the transport shell to store.
    StsPolicy(sts::StsAction),
    /// Server input exceeded a configured state-retention bound.
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// A monitored nickname came online.
    MonitorOnline {
        /// The monitored nickname.
        nick: String,
        /// The user's current username, when supplied by the server.
        user: Option<String>,
        /// The user's current hostname, when supplied by the server.
        host: Option<String>,
    },
    /// A monitored nickname went offline.
    MonitorOffline {
        /// The monitored nickname.
        nick: String,
    },
    /// The server rejected nicknames because its monitor list was full.
    MonitorListFull {
        /// The server's advertised monitor-list limit.
        limit: usize,
        /// Nicknames the server did not add.
        nicks: Vec<String>,
    },
    /// The server does not advertise MONITOR support.
    MonitorUnsupported,
    /// A user joined a channel shared with the session.
    Joined {
        /// The channel that was joined.
        channel: String,
        /// The joining user's nickname.
        nick: String,
    },
    /// Initial channel membership synchronization completed.
    ChannelSynced {
        /// The synchronized channel.
        channel: String,
    },
    /// A user left a shared channel with `PART`.
    Parted {
        /// The departed channel.
        channel: String,
        /// The departing user's nickname.
        nick: String,
        /// The optional part reason.
        reason: Option<String>,
    },
    /// A user was removed from a shared channel with `KICK`.
    Kicked {
        /// The affected channel.
        channel: String,
        /// The removed user's nickname.
        nick: String,
        /// The user or server that issued the kick.
        by: String,
        /// The optional kick reason.
        reason: Option<String>,
    },
    /// A user was invited to a channel.
    Invited {
        /// The invited channel.
        channel: String,
        /// The invited user's nickname.
        nick: String,
        /// The user or server prefix that sent the invitation.
        by: String,
    },
    /// A user sharing channels with the session disconnected.
    Quit {
        /// The disconnected user's nickname.
        nick: String,
        /// The optional quit reason.
        reason: Option<String>,
        /// Shared channels captured before the user was removed.
        channels: Vec<String>,
    },
    /// A user changed nickname.
    NickChanged {
        /// The previous nickname.
        old: String,
        /// The new nickname.
        new: String,
    },
    /// A joined channel was renamed.
    ChannelRenamed {
        /// The previous channel name.
        old: String,
        /// The new channel name.
        new: String,
        /// The server-supplied reason, which may be empty.
        reason: String,
    },
    /// A user's account state changed.
    AccountChanged {
        /// The affected user's nickname.
        nick: String,
        /// The user's new account state.
        account: Account,
    },
    /// A user's away state changed.
    AwayChanged {
        /// The affected user's nickname.
        nick: String,
        /// The away message, or `None` when the user returned.
        message: Option<String>,
    },
    /// A user's username or hostname changed.
    HostChanged {
        /// The affected user's nickname.
        nick: String,
        /// The user's new username.
        user: String,
        /// The user's new hostname.
        host: String,
    },
    /// A user's real name changed.
    RealnameChanged {
        /// The affected user's nickname.
        nick: String,
        /// The user's new real name.
        realname: String,
    },
    /// A user or channel mode changed.
    ModeChanged {
        /// The mode target.
        target: String,
        /// The user or server that changed the modes.
        by: String,
        /// Parsed mode operations in wire order.
        changes: Vec<OwnedModeChange>,
    },
    /// A channel topic changed live.
    TopicChanged {
        /// The affected channel.
        channel: String,
        /// The new topic, or `None` when it was cleared.
        topic: Option<String>,
        /// The user or server that changed the topic.
        by: String,
    },
}

/// Why an incomplete server batch was released as individual messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BatchAbandonReason {
    /// The server opened more batches than configured.
    OpenBatchLimit,
    /// A batch exceeded its configured retained-line limit.
    LineLimit,
    /// A multiline batch exceeded its negotiated text-byte limit.
    ByteLimit,
    /// A batch did not close before its deadline.
    Timeout,
}

/// A parsed message annotated with protocol dispatch context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedMessage {
    message: Message,
    batch_type: Option<String>,
    echo: bool,
    other_session: bool,
}

impl ReceivedMessage {
    /// Borrows the parsed IRC message.
    #[must_use]
    pub fn as_ref(&self) -> MessageRef<'_> {
        self.message.as_ref()
    }

    /// Returns the containing batch's type, when known.
    #[must_use]
    pub fn batch_type(&self) -> Option<&str> {
        self.batch_type.as_deref()
    }

    /// Reports whether the source matches the session's current nickname.
    #[must_use]
    pub const fn is_echo(&self) -> bool {
        self.echo
    }

    /// Reports whether the message came from another session using our nickname.
    #[must_use]
    pub const fn is_other_session(&self) -> bool {
        self.other_session
    }

    /// Removes the dispatch annotation and returns the parsed IRC message.
    #[must_use]
    pub fn into_message(self) -> Message {
        self.message
    }

    /// Splits the message into its wire value and dispatch annotations.
    #[must_use]
    pub fn into_parts(self) -> (Message, Option<String>, bool) {
        (self.message, self.batch_type, self.echo)
    }

    /// Splits the message into its wire value and all dispatch annotations.
    #[must_use]
    pub fn into_context_parts(self) -> (Message, Option<String>, bool, bool) {
        (self.message, self.batch_type, self.echo, self.other_session)
    }
}

impl std::ops::Deref for ReceivedMessage {
    type Target = Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

/// A bounded server-controlled state collection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum StateLimit {
    /// Joined channels.
    Channels,
    /// Members retained for one channel.
    Members,
    /// Targets retained in the metadata cache.
    MetadataTargets,
    /// Metadata keys retained for one target.
    MetadataKeys,
    /// Upstream bouncer networks.
    #[cfg(feature = "unstable")]
    BouncerNetworks,
}

/// Sans-I/O state for one IRC client session.
pub struct Session {
    account_registration: AccountRegistration,
    away_on_connect_sent: bool,
    away_messages: BTreeMap<RequestId, String>,
    batches: Batches,
    config: Config,
    connection: Connection,
    caps: Caps,
    ctcp_sources: VecDeque<(String, Instant)>,
    framer: Framer,
    isupport: IsupportState,
    keepalive: Keepalive,
    monitor: Monitor,
    metadata_subscriptions_sent: bool,
    rate_limit: RateLimiter,
    requests: BTreeMap<String, (RequestId, Instant, bool, Option<Feature>, bool)>,
    unlabelled_requests: VecDeque<UnlabelledRequest>,
    next_request_id: u64,
    registration: Registration,
    sasl: SaslSession,
    state: State,
    state_generation: u64,
    outbox: Outbox,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("account_registration", &self.account_registration)
            .field("away_on_connect_sent", &self.away_on_connect_sent)
            .field("away_messages", &self.away_messages)
            .field("batches", &self.batches)
            .field("config", &self.config)
            .field("connection", &self.connection)
            .field("caps", &self.caps)
            .field("ctcp_sources", &self.ctcp_sources)
            .field("framer", &self.framer)
            .field("isupport", &self.isupport)
            .field("keepalive", &self.keepalive)
            .field("monitor", &self.monitor)
            .field(
                "metadata_subscriptions_sent",
                &self.metadata_subscriptions_sent,
            )
            .field("rate_limit", &self.rate_limit)
            .field("requests", &self.requests)
            .field("unlabelled_requests", &self.unlabelled_requests)
            .field("next_request_id", &self.next_request_id)
            .field("registration", &self.registration)
            .field("sasl", &self.sasl)
            .field("state", &self.state)
            .field("state_generation", &self.state_generation)
            .field("outbox", &self.outbox)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Handled {
    Yes,
    No,
}

#[derive(Debug, Default)]
struct Outbox {
    transmits: VecDeque<Transmission>,
    immediate: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
    close_requested: bool,
}

#[derive(Debug)]
enum Transmission {
    Line(Vec<u8>),
    Batch {
        lines: VecDeque<Vec<u8>>,
        started: bool,
    },
}

impl Transmission {
    fn line_count(&self) -> usize {
        match self {
            Self::Line(_) => 1,
            Self::Batch { lines, .. } => lines.len(),
        }
    }
}

impl Outbox {
    fn transmit_raw(&mut self, line: Vec<u8>) {
        if self.close_requested {
            return;
        }
        if bypasses_rate_limit(&line) {
            self.immediate.push_back(line);
        } else {
            self.transmits.push_back(Transmission::Line(line));
        }
    }

    fn transmit_batch(&mut self, lines: Vec<Vec<u8>>) {
        if !self.close_requested && !lines.is_empty() {
            self.transmits.push_back(Transmission::Batch {
                lines: lines.into(),
                started: false,
            });
        }
    }

    fn transmit<const N: usize>(&mut self, command: &str, params: [&str; N]) {
        match Message::new(command, params).to_line() {
            Ok(line) => self.transmit_raw(line.into()),
            Err(error) => self.close(
                CloseKind::Other,
                format!("invalid outbound message: {error}"),
            ),
        }
    }

    fn transmit_immediate<const N: usize>(&mut self, command: &str, params: [&str; N]) {
        if self.close_requested {
            return;
        }
        match Message::new(command, params).to_line() {
            Ok(line) => self.immediate.push_back(line.into()),
            Err(error) => self.close(
                CloseKind::Other,
                format!("invalid outbound message: {error}"),
            ),
        }
    }

    fn event(&mut self, event: Event) {
        self.events.push_back(event);
    }

    fn close(&mut self, kind: CloseKind, reason: String) {
        self.close_requested = true;
        self.event(Event::Close { kind, reason });
    }

    fn close_and_discard(&mut self, kind: CloseKind, reason: String) {
        self.transmits.clear();
        self.immediate.clear();
        self.close(kind, reason);
    }

    fn reset_connection(&mut self) {
        self.transmits.clear();
        self.immediate.clear();
        self.close_requested = false;
    }
}

impl Session {
    /// Creates a session with no active connection.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self::from_config(config, None)
    }

    /// Creates a session with entropy for authentication mechanisms that need it.
    #[must_use]
    pub fn with_entropy(config: Config, entropy: Box<dyn Entropy + Send>) -> Self {
        Self::from_config(config, Some(entropy))
    }

    fn from_config(config: Config, entropy: Option<Box<dyn Entropy + Send>>) -> Self {
        let connection = Connection {
            host: config.host.clone(),
            security: config.connection_security,
        };
        let registration = Registration::new(&config);
        let isupport = IsupportState::new(config.decoding);
        let sasl = SaslSession::new(&config, entropy);
        let rate_limit = RateLimiter::new(config.rate_limit);
        let state = State::new(&config);
        let batches = Batches::new(&config);
        Self {
            account_registration: AccountRegistration::default(),
            away_on_connect_sent: false,
            away_messages: BTreeMap::new(),
            batches,
            config,
            connection,
            caps: Caps::default(),
            ctcp_sources: VecDeque::new(),
            framer: Framer::default(),
            isupport,
            keepalive: Keepalive::default(),
            monitor: Monitor::default(),
            metadata_subscriptions_sent: false,
            rate_limit,
            requests: BTreeMap::new(),
            unlabelled_requests: VecDeque::new(),
            next_request_id: 0,
            registration,
            sasl,
            state,
            state_generation: 0,
            outbox: Outbox::default(),
        }
    }

    /// Marks the transport as connected at `now`.
    pub fn handle_connected(&mut self, now: Instant) {
        let connection = Connection {
            host: self.config.host.clone(),
            security: self.config.connection_security,
        };
        self.handle_connected_with(now, connection);
    }

    /// Marks a transport with connection-specific context as connected at `now`.
    pub fn handle_connected_with(&mut self, now: Instant, connection: Connection) {
        self.connection = connection;
        self.framer.reset();
        self.outbox.reset_connection();
        self.batches.reset();
        self.ctcp_sources.clear();
        self.requests.clear();
        self.unlabelled_requests.clear();
        self.away_messages.clear();
        self.rate_limit.reset(now);
        self.account_registration.handle_disconnected();
        if let Some(pending) = self.config.registration_on_connect.clone() {
            self.account_registration.begin(pending);
        }
        self.away_on_connect_sent = false;
        self.metadata_subscriptions_sent = false;
        self.caps.handle_connected(now);
        self.isupport.handle_connected();
        self.sasl.handle_connected();
        self.state.reset(&self.config);
        self.state_generation = self.state_generation.wrapping_add(1);
        self.keepalive
            .handle_connected(now, self.config.ping_interval, self.config.ping_timeout);
        self.registration
            .handle_connected(now, &self.config, &mut self.outbox);
    }

    /// Supplies bytes read from a stream transport at `now`.
    pub fn handle_bytes(&mut self, now: Instant, bytes: &[u8]) {
        let frames = self.framer.push(bytes);
        for line in frames.lines {
            self.handle_line(now, &line);
            if self.framer.is_closed() {
                return;
            }
        }
        if frames.too_long {
            self.close_for_long_line();
        }
    }

    /// Supplies one line from a transport that performs its own framing.
    pub fn handle_line(&mut self, now: Instant, line: &[u8]) {
        if self.framer.is_closed() {
            return;
        }
        self.rate_limit.advance(now);
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            return;
        }
        if line.len() > MAX_INCOMING_LINE_LEN {
            self.close_for_long_line();
            return;
        }
        self.keepalive.handle_incoming(now);

        let decoded = match std::str::from_utf8(line) {
            Ok(line) => std::borrow::Cow::Borrowed(line),
            Err(_) if self.isupport.decoding() == Decoding::Utf8OrLatin1 => {
                std::borrow::Cow::Owned(line.iter().map(|&byte| char::from(byte)).collect())
            }
            Err(_) => return self.malformed(line),
        };
        if decoded.contains(['\r', '\n', '\0']) {
            self.malformed(line);
            return;
        }
        match MessageRef::parse(&decoded) {
            Ok(message) => {
                let multiline_max_bytes = outgoing::multiline_limits(self.caps.enabled())
                    .map_or(usize::MAX, |limits| limits.max_bytes);
                match self.batches.handle(now, &message, multiline_max_bytes) {
                    BatchAction::Consumed => {}
                    BatchAction::Complete(events) => {
                        for event in events {
                            self.dispatch_batch_event(now, event);
                        }
                    }
                    BatchAction::Dispatch(context) => {
                        self.dispatch_message(now, &message, &context, true);
                    }
                }
            }
            Err(_) => self.malformed(line),
        }
    }

    /// Notifies the session that its current deadline was reached.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.rate_limit.advance(now);
        for event in self.batches.handle_timeout(now) {
            self.dispatch_batch_event(now, event);
        }
        let expired: Vec<_> = self
            .requests
            .iter()
            .filter(|(_, (_, started, _, _, _))| {
                started
                    .checked_add(self.config.request_timeout)
                    .is_some_and(|deadline| now >= deadline)
            })
            .map(|(label, _)| label.clone())
            .collect();
        for label in expired {
            if let Some((id, _, _, _, _)) = self.requests.remove(&label) {
                self.unlabelled_requests.retain(|request| request.id != id);
                self.away_messages.remove(&id);
                self.outbox.event(Event::RequestTimedOut { id });
            }
        }
        let pending = std::mem::take(&mut self.unlabelled_requests);
        for request in pending {
            if request
                .started
                .checked_add(self.config.request_timeout)
                .is_some_and(|deadline| now >= deadline)
            {
                self.away_messages.remove(&request.id);
                self.outbox.event(Event::RequestTimedOut { id: request.id });
            } else {
                self.unlabelled_requests.push_back(request);
            }
        }
        let was_registered = self.registration.is_registered();
        self.registration.handle_timeout(
            now,
            self.caps.enabled(),
            self.isupport.value(),
            &mut self.outbox,
        );
        if !was_registered && self.registration.is_registered() {
            self.monitor
                .restore(self.isupport.value(), &mut self.outbox);
        }
        self.sasl.handle_timeout(now, &mut self.outbox);
        self.maybe_subscribe_metadata(now);
        let bouncer_network = {
            #[cfg(feature = "unstable")]
            {
                self.config.bouncer_network.as_ref().map(NetworkId::as_str)
            }
            #[cfg(not(feature = "unstable"))]
            None
        };
        self.caps.maybe_finish(
            self.sasl.is_finished() && self.account_registration.authentication_finished(),
            bouncer_network,
            &mut self.outbox,
        );
        self.keepalive.handle_timeout(now, &mut self.outbox);
        self.apply_close_request();
    }

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.framer.close();
        self.outbox.reset_connection();
        self.batches.reset();
        for (_, (id, _, _, _, _)) in std::mem::take(&mut self.requests) {
            self.unlabelled_requests.retain(|request| request.id != id);
            self.outbox.event(Event::RequestAborted { id });
        }
        for request in self.unlabelled_requests.drain(..) {
            self.outbox.event(Event::RequestAborted { id: request.id });
        }
        self.away_messages.clear();
        self.registration.handle_disconnected();
        self.account_registration.handle_disconnected();
        self.sasl.handle_disconnected();
        self.keepalive.handle_disconnected();
        self.monitor.handle_disconnected();
        self.metadata_subscriptions_sent = false;
        self.state.reset(&self.config);
        self.state_generation = self.state_generation.wrapping_add(1);
    }

    /// Validates and queues a user-originated raw message for transmission.
    ///
    /// The caller paces transmission by draining the intentionally unbounded
    /// outbound queue through [`Session::poll_transmit`].
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if `message` cannot be represented as a valid IRC wire line.
    pub fn send(&mut self, now: Instant, message: Message) -> Result<(), BuildError> {
        self.rate_limit.advance(now);
        let line = message.to_line()?;
        drop(message);
        self.outbox.transmit_raw(line.into());
        Ok(())
    }

    fn text_budget(&self, kind: TextKind, target: &str) -> usize {
        outgoing::text_budget(&self.state, kind, target)
    }

    /// Splits and queues text within the server's relayed line budget.
    ///
    /// Context tags (`+reply`, `+draft/reply`, `+channel-context`, and its draft alias) apply
    /// only to the first fallback line. Other supplied tags are copied to every line.
    /// Each client batch is one atomic queue entry, so no other line is transmitted inside it.
    /// A missing, non-numeric, or zero `max-bytes` value disables multiline; a missing
    /// `max-lines` value permits any number of lines, while a zero value disables multiline.
    /// A `FAIL BATCH MULTILINE_*` is surfaced as an error or reply without automatic retry.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::EmptyText`] when `text` has no nonempty lines, or the validation
    /// and pending-request errors documented by [`Session::request`].
    pub fn send_text(
        &mut self,
        now: Instant,
        kind: TextKind,
        target: &str,
        text: &str,
        tags: &[(&str, &str)],
    ) -> Result<Option<RequestId>, RequestError> {
        self.send_text_detailed(now, kind, target, text, tags)
            .map(|sent| sent.request)
    }

    /// Queues text and reports whether line splitting was applied.
    #[doc(hidden)]
    pub fn send_text_detailed(
        &mut self,
        now: Instant,
        kind: TextKind,
        target: &str,
        text: &str,
        tags: &[(&str, &str)],
    ) -> Result<SendTextResult, RequestError> {
        self.rate_limit.advance(now);
        let multiline = self
            .caps
            .enabled()
            .contains_key("batch")
            .then(|| outgoing::multiline_limits(self.caps.enabled()))
            .flatten();
        let budget = multiline.map_or(self.text_budget(kind, target), |limits| {
            self.text_budget(kind, target).min(limits.max_bytes)
        });
        let segments = papo_proto::split_text(text, budget);
        if segments.is_empty() {
            return Err(RequestError::EmptyText);
        }
        let labelled = self.caps.enabled().contains_key("labeled-response");
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        let id = RequestId(self.next_request_id);
        let label = labelled.then(|| encode_label(self.next_request_id));
        let mut lines = Vec::new();
        let mut batch_count = 0;
        if let Some(limits) = multiline.filter(|_| text.contains('\n') || segments.len() > 1) {
            let mut start = 0;
            while start < segments.len() {
                let mut end = start;
                let mut bytes = 0;
                while end < segments.len() && end - start < limits.max_lines {
                    let separator = usize::from(end > start && !segments[end - 1].concat);
                    if bytes + separator + segments[end].text.len() > limits.max_bytes {
                        break;
                    }
                    bytes += separator + segments[end].text.len();
                    end += 1;
                }
                let id = format!("b{}-{batch_count}", encode_label(self.next_request_id));
                let params = [format!("+{id}"), "draft/multiline".into(), target.into()];
                let mut open = Message::new("BATCH", params);
                for &(key, value) in tags {
                    if batch_count == 0 || !outgoing::first_line_only(key) {
                        open = open.with_tag(key, value);
                    }
                }
                if batch_count == 0
                    && let Some(label) = &label
                {
                    open = open.with_tag("label", label);
                }
                lines.push(open.to_line()?.into_bytes());
                for (offset, segment) in segments[start..end].iter().enumerate() {
                    let mut inner =
                        Message::new(kind.command(), [target, segment.text]).with_tag("batch", &id);
                    if offset > 0 && segments[offset + start - 1].concat {
                        inner = inner.with_tag("draft/multiline-concat", "");
                    }
                    lines.push(inner.to_line()?.into_bytes());
                }
                lines.push(
                    Message::new("BATCH", [format!("-{id}")])
                        .to_line()?
                        .into_bytes(),
                );
                start = end;
                batch_count += 1;
            }
        } else {
            for (index, segment) in segments.iter().enumerate() {
                let mut message = Message::new(kind.command(), [target, segment.text]);
                for &(key, value) in tags {
                    if index == 0 || !outgoing::first_line_only(key) {
                        message = message.with_tag(key, value);
                    }
                }
                if index == 0
                    && let Some(label) = &label
                {
                    message = message.with_tag("label", label);
                }
                lines.push(message.to_line()?.into_bytes());
            }
        }
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if let Some(label) = label {
            let sent = self.caps.enabled().contains_key("echo-message");
            self.requests.insert(label, (id, now, sent, None, true));
        }
        if batch_count == 0 {
            for line in lines {
                self.outbox.transmit_raw(line);
            }
        } else {
            self.outbox.transmit_batch(lines);
        }
        Ok(SendTextResult {
            request: labelled.then_some(id),
            split: segments.len() > 1,
        })
    }

    /// Queues a message and assigns an identifier for its eventual response.
    ///
    /// When `labeled-response` is enabled, the message receives a fresh `label` tag and remains
    /// pending. Otherwise it is sent unchanged and [`Event::RequestUntracked`] is emitted.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when `labeled-response` or a feature explicitly
    /// requested by the message is unavailable,
    /// [`RequestError::PendingLimit`] when 256 labelled requests are already pending, or
    /// [`RequestError::Build`] when the tagged message cannot be represented on the wire.
    pub fn request(&mut self, now: Instant, message: Message) -> Result<RequestId, RequestError> {
        self.request_with_event_policy(now, message, true)
    }

    /// Queues a request whose response updates state without emitting its lines as events.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when `labeled-response` or a feature explicitly
    /// requested by the message is unavailable,
    /// [`RequestError::PendingLimit`] when 256 labelled requests are already pending, or
    /// [`RequestError::Build`] when the tagged message cannot be represented on the wire.
    pub fn request_silent(
        &mut self,
        now: Instant,
        message: Message,
    ) -> Result<RequestId, RequestError> {
        self.request_with_event_policy(now, message, false)
    }

    fn request_with_event_policy(
        &mut self,
        now: Instant,
        message: Message,
        emit_events: bool,
    ) -> Result<RequestId, RequestError> {
        self.require_feature(Feature::LabeledResponse)?;
        let feature = requested_message_feature(&message.as_ref());
        if let Some(feature) = feature {
            self.require_feature(feature)?;
        }
        self.queue_request(
            now,
            message,
            Some(feature.unwrap_or(Feature::LabeledResponse)),
            emit_events,
        )
    }

    pub(crate) fn request_for_feature(
        &mut self,
        now: Instant,
        message: Message,
        feature: Feature,
    ) -> Result<RequestId, RequestError> {
        self.require_feature(feature)?;
        self.queue_request(now, message, Some(feature), true)
    }

    fn queue_request(
        &mut self,
        now: Instant,
        mut message: Message,
        feature: Option<Feature>,
        emit_events: bool,
    ) -> Result<RequestId, RequestError> {
        self.rate_limit.advance(now);
        let id = RequestId(self.next_request_id);
        let sent = self.caps.enabled().contains_key("echo-message")
            && matches!(
                message.as_ref().command(),
                Command::Privmsg | Command::Notice
            );
        let labelled = self.caps.enabled().contains_key("labeled-response");
        let label = labelled.then(|| encode_label(self.next_request_id));
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        if let Some(label) = &label {
            message = message.with_tag("label", label);
        }
        let line = message.to_line()?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if let Some(label) = label {
            self.requests
                .insert(label, (id, now, sent, feature, emit_events));
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox.transmit_raw(line.into());
        Ok(id)
    }

    /// Queues a request with a fallback response boundary for unlabelled servers.
    ///
    /// Label correlation is still preferred whenever `labeled-response` is enabled.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::PendingLimit`] when 256 requests are already pending, or
    /// [`RequestError::Build`] when the message cannot be represented on the wire.
    pub fn request_with_matcher(
        &mut self,
        now: Instant,
        message: Message,
        matcher: ResponseMatcher,
    ) -> Result<RequestId, RequestError> {
        let away_message = matches!(matcher, ResponseMatcher::Away(true))
            .then(|| message.as_ref().param(0).map(str::to_owned))
            .flatten();
        if self.caps.enabled().contains_key("labeled-response") {
            let track_sync = matches!(matcher, ResponseMatcher::Join(_));
            if track_sync && self.unlabelled_requests.len() >= MAX_PENDING_REQUESTS {
                return Err(RequestError::PendingLimit);
            }
            let id = self.queue_request(now, message, None, true)?;
            if let Some(message) = away_message {
                self.away_messages.insert(id, message);
            }
            if track_sync {
                self.unlabelled_requests.push_back(UnlabelledRequest {
                    id,
                    started: now,
                    matcher,
                    fence: None,
                    lines: Vec::new(),
                });
            }
            return Ok(id);
        }
        self.rate_limit.advance(now);
        if self.unlabelled_requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        let line = message.to_line()?;
        let id = RequestId(self.next_request_id);
        let fence = matches!(matcher, ResponseMatcher::Mode(_))
            .then(|| format!("papo-mode-{}", self.next_request_id));
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.unlabelled_requests.push_back(UnlabelledRequest {
            id,
            started: now,
            matcher,
            fence: fence.clone(),
            lines: Vec::new(),
        });
        if let Some(message) = away_message {
            self.away_messages.insert(id, message);
        }
        self.outbox.transmit_raw(line.into());
        if let Some(fence) = fence {
            self.outbox
                .transmit_raw(format!("PING :{fence}\r\n").into_bytes());
        }
        Ok(id)
    }

    /// Adds nicknames to the desired server-side monitor list.
    pub fn monitor_add<I, S>(&mut self, nicks: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.monitor.add(
            nicks,
            self.isupport.value(),
            self.registration.is_registered(),
            &mut self.outbox,
        );
    }

    /// Removes nicknames from the desired server-side monitor list.
    pub fn monitor_remove<I, S>(&mut self, nicks: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let before = self.state.clone();
        self.monitor.remove(
            nicks,
            self.isupport.value(),
            self.registration.is_registered(),
            &mut self.outbox,
        );
        self.sync_monitored_state();
        if before != self.state {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    /// Clears the desired server-side monitor list.
    pub fn monitor_clear(&mut self) {
        let before = self.state.clone();
        let supported =
            self.registration.is_registered() && self.isupport.value().monitor().is_some();
        self.monitor.clear(supported, &mut self.outbox);
        self.sync_monitored_state();
        if before != self.state {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    /// Returns the next CRLF-terminated wire line to transmit.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        let batch_open = matches!(
            self.outbox.transmits.front(),
            Some(Transmission::Batch { started: true, .. })
        );
        if !batch_open && let Some(line) = self.outbox.immediate.pop_front() {
            return Some(line);
        }
        if self.outbox.transmits.is_empty()
            || !self.rate_limit.take(self.isupport.value().saferate())
        {
            return None;
        }
        match self.outbox.transmits.pop_front()? {
            Transmission::Line(line) => Some(line),
            Transmission::Batch { mut lines, .. } => {
                let line = lines.pop_front();
                if !lines.is_empty() {
                    self.outbox.transmits.push_front(Transmission::Batch {
                        lines,
                        started: true,
                    });
                }
                line
            }
        }
    }

    /// Returns the next session event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.outbox.events.pop_front()
    }

    /// Returns the next protocol deadline, if any.
    #[must_use]
    pub fn poll_timeout(&self) -> Option<Instant> {
        [
            self.registration.next_deadline(),
            self.sasl.next_deadline(),
            self.keepalive.next_deadline(),
            self.rate_limit.next_deadline(
                !self.outbox.transmits.is_empty(),
                self.isupport.value().saferate(),
            ),
            self.batches.next_deadline(),
            self.requests
                .values()
                .filter_map(|(_, started, _, _, _)| {
                    started.checked_add(self.config.request_timeout)
                })
                .min(),
            self.unlabelled_requests
                .iter()
                .filter_map(|request| request.started.checked_add(self.config.request_timeout))
                .min(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Returns the nickname currently assigned to this session.
    #[must_use]
    pub fn nick(&self) -> &str {
        self.registration.nick()
    }

    /// Returns the `IRCv3` capabilities enabled on this connection.
    #[must_use]
    pub fn caps(&self) -> &Capabilities {
        self.caps.enabled()
    }

    /// Returns the limits advertised for the negotiated metadata capability.
    #[must_use]
    pub fn metadata_limits(&self) -> Option<MetadataLimits> {
        MetadataLimits::from_caps(self.caps.enabled())
    }

    /// Returns the accumulated server features advertised through `005`.
    #[must_use]
    pub fn isupport(&self) -> &papo_proto::Isupport {
        self.isupport.value()
    }

    /// Returns the session's channel and user state.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// Reports whether the server will restore joined channels for this account.
    #[must_use]
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn server_rejoins_channels(&self) -> bool {
        self.state
            .persistence()
            .is_some_and(|status| status.effective)
    }

    /// Returns a counter that changes when snapshot-visible state changes.
    #[must_use]
    pub const fn state_generation(&self) -> u64 {
        self.state_generation
    }

    /// Returns the identifier that the next tracked request will receive.
    #[doc(hidden)]
    #[must_use]
    pub const fn next_request_id(&self) -> RequestId {
        RequestId(self.next_request_id)
    }

    /// Reports whether connection registration has completed.
    #[must_use]
    pub const fn is_registered(&self) -> bool {
        self.registration.is_registered()
    }

    /// Returns the account name reported by the server, when authenticated.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account_registration
            .account()
            .or_else(|| self.sasl.account())
    }

    /// Requests creation of an account through `draft/account-registration`.
    ///
    /// An omitted account or email address is encoded as `*`.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] before connection registration or when the
    /// capability was not negotiated, or [`RequestError::InvalidSecret`] when `password`
    /// contains bytes rather than text. Other failures are the same as [`Session::request`].
    pub fn register(
        &mut self,
        now: Instant,
        account: Option<&str>,
        email: Option<&str>,
        password: impl Into<Secret>,
    ) -> Result<(), RequestError> {
        let registered = self.registration.is_registered();
        let value = if registered {
            self.caps
                .enabled()
                .get("draft/account-registration")
                .map(Option::as_deref)
        } else {
            self.caps
                .advertised_value("draft/account-registration")
                .map(Option::as_deref)
        };
        let Some(value) = value else {
            return self.require_feature(Feature::AccountRegistration);
        };
        let policy = RegistrationPolicy::parse(value);
        if !registered && !policy.before_connect {
            return self.require_feature(Feature::AccountRegistration);
        }
        if email.is_none() && policy.email_required {
            return Err(RequestError::EmailRequired);
        }
        if account.is_some_and(|account| {
            !policy.custom_account_name
                && !self.isupport.value().casemapping().eq(account, self.nick())
        }) {
            return Err(RequestError::CustomAccountNameUnsupported);
        }
        let password = password.into();
        if !registered {
            if !self.caps.is_pending("draft/account-registration") {
                return self.require_feature(Feature::AccountRegistration);
            }
            self.account_registration.begin(PendingRegistration {
                account: account.map(str::to_owned),
                email: email.map(str::to_owned),
                password,
            });
            return Ok(());
        }
        self.send_register(now, account, email, &password)
    }

    fn send_register(
        &mut self,
        now: Instant,
        account: Option<&str>,
        email: Option<&str>,
        password: &Secret,
    ) -> Result<(), RequestError> {
        self.request_for_feature(
            now,
            Message::new(
                "REGISTER",
                [
                    account.unwrap_or("*"),
                    email.unwrap_or("*"),
                    password.expose().ok_or(RequestError::InvalidSecret)?,
                ],
            ),
            Feature::AccountRegistration,
        )?;
        self.account_registration.request_started();
        Ok(())
    }

    /// Submits a verification code for `draft/account-registration`.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when the capability was not negotiated. Other
    /// failures are the same as [`Session::request`].
    pub fn verify(&mut self, now: Instant, account: &str, code: &str) -> Result<(), RequestError> {
        self.require_feature(Feature::AccountRegistration)?;
        if !self.registration.is_registered()
            && !RegistrationPolicy::parse(
                self.caps
                    .enabled()
                    .get("draft/account-registration")
                    .and_then(Option::as_deref),
            )
            .before_connect
        {
            return Err(RequestError::unsupported(Feature::AccountRegistration));
        }
        self.request_for_feature(
            now,
            Message::new("VERIFY", [account, code]),
            Feature::AccountRegistration,
        )?;
        self.account_registration.request_started();
        Ok(())
    }

    /// Returns the most recently measured keepalive round-trip time.
    #[must_use]
    pub const fn lag(&self) -> Option<Duration> {
        self.keepalive.lag()
    }

    /// Returns the number of lines waiting to be transmitted.
    ///
    /// The caller paces transmission by draining this intentionally unbounded
    /// queue through [`Session::poll_transmit`].
    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.outbox.immediate.len()
            + self
                .outbox
                .transmits
                .iter()
                .map(Transmission::line_count)
                .sum::<usize>()
    }

    /// Starts another SASL exchange with the configured credentials.
    pub fn reauthenticate(&mut self, now: Instant) {
        let available = self.caps.enabled().contains_key("sasl");
        self.sasl.reauthenticate(
            now,
            available,
            self.caps.enabled().get("sasl").and_then(Option::as_deref),
            &mut self.outbox,
        );
        self.apply_close_request();
    }

    fn malformed(&mut self, line: &[u8]) {
        self.outbox.event(Event::Malformed {
            line: line.to_vec(),
        });
    }

    fn maybe_send_away_on_connect(&mut self) {
        if self.away_on_connect_sent {
            return;
        }
        let pre_away = self.caps.enabled().contains_key("draft/pre-away");
        if !pre_away && !self.registration.is_registered() {
            return;
        }
        let parameter = match &self.config.away_on_connect {
            Some(AwayOnConnect::Message(message)) => message.as_str(),
            Some(AwayOnConnect::Hidden) if pre_away => "*",
            Some(AwayOnConnect::Hidden) | None => {
                self.away_on_connect_sent = true;
                return;
            }
        };
        self.outbox.transmit_immediate("AWAY", [parameter]);
        self.away_on_connect_sent = true;
    }

    fn snapshot_values(&self) -> (State, Capabilities, papo_proto::Isupport, Option<String>) {
        (
            self.state.clone(),
            self.caps.enabled().clone(),
            self.isupport.value().clone(),
            self.account().map(str::to_owned),
        )
    }

    fn snapshot_values_for(
        &self,
        message: &MessageRef<'_>,
    ) -> (
        StateSnapshot,
        Capabilities,
        papo_proto::Isupport,
        Option<String>,
    ) {
        (
            self.state.snapshot_for(message),
            self.caps.enabled().clone(),
            self.isupport.value().clone(),
            self.account().map(str::to_owned),
        )
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch_message(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        context: &DispatchContext,
        emit_unhandled: bool,
    ) {
        let queued_events = self.outbox.events.len();
        let emit_response_events = self.response_emits_events(message);
        let before = self.snapshot_values_for(message);
        let was_registered = self.registration.is_registered();
        let before_features = was_registered.then(|| self.supported_features());
        let registration_handled = self.registration.handle_message(
            now,
            message,
            &self.config,
            self.caps.enabled(),
            self.isupport.value(),
            &mut self.outbox,
        ) == Handled::Yes;
        let caps_handled = self.caps.handle_message(
            now,
            message,
            &self.config,
            &self.connection,
            &mut self.sasl,
            &mut self.outbox,
        ) == Handled::Yes;
        let sasl_handled = self.sasl.handle_message(now, message, &mut self.outbox) == Handled::Yes;
        let account_registration_handled = self
            .account_registration
            .handle_message(message, &mut self.outbox)
            == Handled::Yes;
        self.maybe_send_away_on_connect();
        let isupport_handled = self.isupport.handle_message(message) == Handled::Yes;
        let registration_finished = !was_registered && self.registration.is_registered();
        if registration_finished {
            self.monitor
                .restore(self.isupport.value(), &mut self.outbox);
        }
        let monitor_handled =
            self.monitor
                .handle_message(message, self.isupport.value(), &mut self.outbox)
                == Handled::Yes;
        self.send_pending_registration(now);
        if registration_finished || isupport_handled {
            self.sync_monitored_state();
        } else if monitor_handled {
            for nick in monitor::presence_nicks(message) {
                self.state
                    .sync_monitored_nick(&self.monitor, self.isupport.value(), nick);
            }
        }
        let whoami = self.handle_whoami(message);
        let own_away_message = self.confirmed_away_message(message).map(str::to_owned);
        let state_handled = whoami
            || self.state.handle_message(
                message,
                self.registration.nick(),
                self.isupport.value(),
                self.no_implicit_names(),
                MetadataContext {
                    max_keys: state::MAX_METADATA_KEYS_PER_TARGET,
                    subscription_batch: context.batch_type() == Some("metadata-subs"),
                },
                own_away_message.as_deref(),
                &mut self.outbox,
            ) == Handled::Yes;
        self.handle_ctcp(now, message);
        let handled = registration_handled
            || caps_handled
            || sasl_handled
            || account_registration_handled
            || monitor_handled
            || state_handled
            || isupport_handled
            || self
                .keepalive
                .handle_message(now, message, &mut self.outbox)
                == Handled::Yes;
        let labelled_echo = matches!(message.command(), Command::Privmsg | Command::Notice)
            && message.tag("label").is_some_and(|label| {
                self.requests
                    .get(label.as_ref())
                    .is_some_and(|(_, _, sent, _, _)| *sent)
            })
            && self.is_own_source(message);
        let labelled_response =
            self.resolve_labelled_message(message, queued_events, emit_response_events);
        let response = self.resolve_unlabelled(message) || labelled_response;
        if response && labelled_echo && emit_response_events {
            self.enqueue_labelled_echo(message);
        }
        let tag_message = !response && emit_unhandled && self.report_tag_message(message);
        let redacted =
            !response && !tag_message && emit_unhandled && self.report_redaction(message);
        let invited =
            !response && !tag_message && !redacted && emit_unhandled && self.report_invite(message);
        let standard_reply = !response
            && !tag_message
            && !redacted
            && !invited
            && emit_unhandled
            && self.report_standard_reply(message);
        self.maybe_subscribe_metadata(now);
        let bouncer_network = {
            #[cfg(feature = "unstable")]
            {
                self.config.bouncer_network.as_ref().map(NetworkId::as_str)
            }
            #[cfg(not(feature = "unstable"))]
            None
        };
        self.caps.maybe_finish(
            self.sasl.is_finished() && self.account_registration.authentication_finished(),
            bouncer_network,
            &mut self.outbox,
        );
        if !handled
            && !response
            && !tag_message
            && !redacted
            && !invited
            && !standard_reply
            && emit_unhandled
        {
            let received = self.received_message(
                message.to_owned(),
                context.batch_type().map(str::to_owned),
                false,
            );
            self.outbox.event(Event::Message(received));
        }
        self.apply_close_request();
        self.report_feature_changes(before_features);
        if before != self.snapshot_values_for(message) {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    fn report_feature_changes(
        &mut self,
        before_features: Option<std::collections::BTreeSet<Feature>>,
    ) {
        let Some(before_features) = before_features else {
            return;
        };
        let after_features = self.supported_features();
        let added: Vec<_> = after_features
            .difference(&before_features)
            .copied()
            .collect();
        let removed: Vec<_> = before_features
            .difference(&after_features)
            .copied()
            .collect();
        if !added.is_empty() || !removed.is_empty() {
            self.outbox.event(Event::FeaturesChanged {
                added,
                removed: removed.clone(),
            });
            for feature in removed {
                let reason = self
                    .supports(feature)
                    .expect_err("a removed feature is unavailable");
                self.fail_requests_for_feature(feature, reason);
            }
        }
    }

    fn fail_requests_for_feature(&mut self, feature: Feature, reason: UnsupportedReason) {
        let labels: Vec<_> = self
            .requests
            .iter()
            .filter(|(_, (_, _, _, required, _))| *required == Some(feature))
            .map(|(label, _)| label.clone())
            .collect();
        for label in labels {
            let (id, _, _, _, _) = self
                .requests
                .remove(&label)
                .expect("the pending label was just selected");
            self.unlabelled_requests.retain(|pending| pending.id != id);
            self.outbox.event(Event::RequestUnsupported {
                id,
                unsupported: Unsupported::new(feature, reason),
            });
        }
    }

    fn enqueue_labelled_echo(&mut self, message: &MessageRef<'_>) {
        let response = self
            .outbox
            .events
            .pop_back()
            .expect("a resolved request queued its response");
        debug_assert!(matches!(response, Event::Response(_)));
        let received = self.received_message(message.to_owned(), None, true);
        self.outbox.event(Event::Message(received));
        // Consumers must observe the confirming event before the action completes.
        self.outbox.event(response);
    }

    fn send_pending_registration(&mut self, now: Instant) {
        if self
            .caps
            .enabled()
            .contains_key("draft/account-registration")
        {
            if let Some(pending) = self.account_registration.take_pending() {
                _ = self.send_register(
                    now,
                    pending.account.as_deref(),
                    pending.email.as_deref(),
                    &pending.password,
                );
            }
        } else if self.caps.is_negotiating() && !self.caps.is_pending("draft/account-registration")
        {
            self.account_registration.cancel_pending();
        }
    }

    fn handle_whoami(&mut self, message: &MessageRef<'_>) -> bool {
        if !self.caps.enabled().contains_key("draft/whoami")
            || !self.registration.is_in_welcome_burst()
            || !self.state.handle_whoami(
                message,
                self.registration.nick(),
                self.isupport.value(),
                &mut self.outbox,
            )
        {
            return false;
        }
        if let Some(Source::User { nick, .. }) = message.source() {
            self.registration.update_nick(nick);
        }
        true
    }

    fn handle_ctcp(&mut self, now: Instant, message: &MessageRef<'_>) -> bool {
        let (Some(Source::User { nick, .. }), Some(text)) = (message.source(), message.param(1))
        else {
            return false;
        };
        let Some(query) = message
            .command_is("PRIVMSG")
            .then(|| Ctcp::parse(text))
            .flatten()
        else {
            return false;
        };
        let response = match query.command.to_ascii_uppercase().as_str() {
            "VERSION" => self.config.ctcp.version.as_deref(),
            "PING" if self.config.ctcp.ping => query.params,
            "TIME" => self.config.ctcp.time.as_deref(),
            "SOURCE" => self.config.ctcp.source.as_deref(),
            _ => None,
        };
        let Some(response) = response else {
            return true;
        };
        self.ctcp_sources.retain(|(_, replied)| {
            replied
                .checked_add(CTCP_REPLY_INTERVAL)
                .is_some_and(|deadline| now < deadline)
        });
        if self.ctcp_sources.iter().any(|(source, _)| source == nick) {
            return true;
        }
        if self.ctcp_sources.len() == MAX_CTCP_SOURCES {
            self.ctcp_sources.pop_front();
        }
        self.ctcp_sources.push_back((nick.to_owned(), now));
        if let Some(response) = Ctcp::build(query.command, Some(response)) {
            self.outbox.transmit("NOTICE", [nick, response.as_str()]);
        }
        true
    }

    fn report_tag_message(&mut self, message: &MessageRef<'_>) -> bool {
        let Some(target) = message.param(0).filter(|_| message.command_is("TAGMSG")) else {
            return false;
        };
        self.outbox.event(Event::TagMessage(TagMessage {
            source: message.raw_source().map(str::to_owned),
            target: target.to_owned(),
            tags: message
                .tags()
                .map(|(key, value)| (key.to_owned(), value.map(std::borrow::Cow::into_owned)))
                .collect(),
        }));
        true
    }

    fn report_redaction(&mut self, message: &MessageRef<'_>) -> bool {
        let (Some(target), Some(msgid)) = (
            message.param(0).filter(|_| message.command_is("REDACT")),
            message.param(1),
        ) else {
            return false;
        };
        self.outbox.event(Event::Redacted(Redacted {
            source: message.raw_source().map(str::to_owned),
            target: target.to_owned(),
            msgid: msgid.to_owned(),
            reason: message.param(2).map(str::to_owned),
        }));
        true
    }

    fn report_invite(&mut self, message: &MessageRef<'_>) -> bool {
        let (Some(nick), Some(channel), Some(by)) = (
            message.param(0).filter(|_| message.command_is("INVITE")),
            message.param(1),
            message.raw_source(),
        ) else {
            return false;
        };
        self.outbox.event(Event::Invited {
            channel: channel.to_owned(),
            nick: nick.to_owned(),
            by: by.to_owned(),
        });
        true
    }

    fn resolve_message(&mut self, message: &MessageRef<'_>) -> bool {
        let Some(label) = message.tag("label") else {
            return false;
        };
        let Some((id, _, sent, _, _)) = self.requests.remove(label.as_ref()) else {
            return false;
        };
        self.away_messages.remove(&id);
        let lines = if message.command_is("ACK") {
            Vec::new()
        } else {
            vec![message.to_owned()]
        };
        let whois_channels = parse_whois_channels(&lines, self.isupport.value());
        if !self
            .unlabelled_requests
            .iter()
            .any(|request| request.id == id)
        {
            self.outbox.event(Event::Response(Response {
                id,
                sent: if sent { sent_info(message) } else { None },
                lines,
                history: None,
                history_targets: None,
                whois_channels,
            }));
        }
        true
    }

    fn resolve_labelled_message(
        &mut self,
        message: &MessageRef<'_>,
        queued_events: usize,
        emit_events: bool,
    ) -> bool {
        let resolved = self.resolve_message(message);
        if resolved && !emit_events {
            let response = self
                .outbox
                .events
                .pop_back()
                .expect("a resolved request queued its response");
            self.outbox.events.truncate(queued_events);
            self.outbox.event(response);
        }
        resolved
    }

    fn response_emits_events(&self, message: &MessageRef<'_>) -> bool {
        message
            .tag("label")
            .and_then(|label| self.requests.get(label.as_ref()))
            .is_none_or(|(_, _, _, _, emit_events)| *emit_events)
    }

    fn confirmed_away_message<'a>(&'a self, message: &MessageRef<'_>) -> Option<&'a str> {
        if message.command() != Command::Numeric(Numeric::RPL_NOWAWAY) {
            return None;
        }
        let id = if let Some(label) = message.tag("label") {
            self.requests.get(label.as_ref()).map(|(id, ..)| *id)
        } else {
            self.unlabelled_requests.iter().find_map(|request| {
                matches!(&request.matcher, ResponseMatcher::Away(true)).then_some(request.id)
            })
        }?;
        self.away_messages.get(&id).map(String::as_str)
    }

    fn resolve_unlabelled(&mut self, message: &MessageRef<'_>) -> bool {
        let mapping = self.isupport.value().casemapping();
        let Some((index, complete)) =
            self.unlabelled_requests
                .iter()
                .enumerate()
                .find_map(|(index, request)| {
                    let fence = request.fence.as_ref().and_then(|token| {
                        (message.command() == Command::Pong
                            && message.params().last() == Some(token.as_str()))
                        .then_some(true)
                    });
                    fence
                        .or_else(|| {
                            request
                                .matcher
                                .matches(message, mapping, self.registration.nick(), &self.state)
                                .map(|complete| complete && request.fence.is_none())
                        })
                        .map(|done| (index, done))
                })
        else {
            return false;
        };
        let request = self.unlabelled_requests.get_mut(index).unwrap();
        request.lines.push(message.to_owned());
        if complete {
            let request = self.unlabelled_requests.remove(index).unwrap();
            self.away_messages.remove(&request.id);
            let whois_channels = parse_whois_channels(&request.lines, self.isupport.value());
            self.outbox.event(Event::Response(Response {
                id: request.id,
                sent: None,
                lines: request.lines,
                history: None,
                history_targets: None,
                whois_channels,
            }));
        }
        true
    }

    fn report_standard_reply(&mut self, message: &MessageRef<'_>) -> bool {
        let Some(reply) = StandardReplyView::parse(message) else {
            return false;
        };
        self.outbox.event(Event::StandardReply(StandardReply {
            kind: reply.kind,
            command: reply.command.to_ascii_uppercase(),
            code: reply.code.to_ascii_uppercase(),
            context: reply.context.into_iter().map(str::to_owned).collect(),
            description: reply.description.to_owned(),
        }));
        true
    }

    fn dispatch_batch_event(&mut self, now: Instant, event: BatchEvent) {
        match event {
            event @ BatchEvent::Response { .. } => self.dispatch_response_batch(now, event),
            BatchEvent::History { target, items, end } => {
                self.outbox.event(Event::History { target, items, end });
            }
            BatchEvent::HistoryTargets(targets) => {
                self.outbox.event(Event::HistoryTargets(targets));
            }
            BatchEvent::Netsplit { servers, quits } => {
                // History can describe stale membership, while netsplit and netjoin batches
                // report connectivity changes that define the current live membership.
                for message in &quits {
                    self.apply_aggregate_state(message);
                }
                self.outbox.event(Event::Netsplit { servers, quits });
            }
            BatchEvent::Netjoin { servers, joins } => {
                for message in &joins {
                    self.apply_aggregate_state(message);
                }
                self.outbox.event(Event::Netjoin { servers, joins });
            }
            BatchEvent::Abandoned {
                reference,
                batch_type,
                reason,
            } => self.outbox.event(Event::BatchAbandoned {
                reference,
                batch_type,
                reason,
            }),
            BatchEvent::Message {
                message,
                batch_type,
            } => {
                let received = self.received_message(message, batch_type, false);
                self.outbox.event(Event::Message(received));
            }
            BatchEvent::Multiline(message) => self
                .outbox
                .event(Event::MultilineMessage(MultilineMessage { message })),
            BatchEvent::Metadata { kind, messages } => {
                if kind == "metadata-subs" {
                    self.state.clear_metadata_subscriptions();
                }
                let context = DispatchContext::from_batch_type(kind);
                for message in messages {
                    self.dispatch_message(now, &message.as_ref(), &context, true);
                }
            }
            #[cfg(feature = "unstable")]
            BatchEvent::Bouncer(messages) => {
                let context = DispatchContext::from_batch_type("soju.im/bouncer-networks".into());
                for message in messages {
                    self.dispatch_message(now, &message.as_ref(), &context, true);
                }
            }
            BatchEvent::Isupport(messages) => {
                let before = self.snapshot_values();
                self.isupport.handle_batch(&messages);
                self.sync_monitored_state();
                if before != self.snapshot_values() {
                    self.state_generation = self.state_generation.wrapping_add(1);
                }
            }
        }
    }

    fn dispatch_response_batch(&mut self, now: Instant, response: BatchEvent) {
        let BatchEvent::Response {
            label,
            echo,
            messages,
            batch_type,
            history,
            history_targets,
        } = response
        else {
            unreachable!("response dispatcher received another batch event");
        };
        let request = self.requests.remove(&label);
        if let Some((id, _, _, _, _)) = &request {
            self.away_messages.remove(id);
        }
        let deferred = request.as_ref().is_some_and(|(id, _, _, _, _)| {
            self.unlabelled_requests
                .iter()
                .any(|pending| pending.id == *id)
        });
        let emit_events = request
            .as_ref()
            .is_none_or(|(_, _, _, _, emit_events)| *emit_events);
        let queued_events = self.outbox.events.len();
        let bouncer_batch = batch_type == "soju.im/bouncer-networks";
        let context = DispatchContext::from_batch_type(batch_type);
        if (history.is_none() || request.is_none()) && !bouncer_batch {
            for message in &messages {
                self.dispatch_message(now, &message.as_ref(), &context, request.is_none());
            }
        }
        if !emit_events {
            self.outbox.events.truncate(queued_events);
        }
        if let Some((id, _, sent, _, _)) = request
            && !deferred
        {
            let whois_channels = parse_whois_channels(&messages, self.isupport.value());
            let echoed = (sent && emit_events).then(|| {
                messages
                    .iter()
                    .filter(|message| {
                        matches!(
                            message.as_ref().command(),
                            Command::Privmsg | Command::Notice
                        ) && self.is_own_source(&message.as_ref())
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            });
            self.outbox.event(Event::Response(Response {
                id,
                sent: if sent {
                    echo.as_ref()
                        .and_then(|message| sent_info(&message.as_ref()))
                        .or_else(|| {
                            messages
                                .iter()
                                .find_map(|message| sent_info(&message.as_ref()))
                        })
                } else {
                    None
                },
                lines: messages,
                history,
                history_targets,
                whois_channels,
            }));
            for message in echoed.into_iter().flatten() {
                let received = self.received_message(message, None, true);
                self.outbox.event(Event::Message(received));
            }
        }
    }

    fn is_own_source(&self, message: &MessageRef<'_>) -> bool {
        message.source().is_some_and(|source| {
            let papo_proto::Source::User { nick, .. } = source else {
                return false;
            };
            self.isupport
                .value()
                .casemapping()
                .eq(nick, self.registration.nick())
        })
    }

    fn received_message(
        &self,
        message: Message,
        batch_type: Option<String>,
        labelled_echo: bool,
    ) -> ReceivedMessage {
        let own_source = self.is_own_source(&message.as_ref());
        let labels = self.caps.enabled().contains_key("labeled-response");
        ReceivedMessage {
            message,
            batch_type,
            echo: own_source && (labelled_echo || !labels),
            other_session: own_source && !labelled_echo && labels,
        }
    }

    fn apply_aggregate_state(&mut self, message: &Message) {
        let before = self.state.clone();
        let event_count = self.outbox.events.len();
        self.state.handle_message(
            &message.as_ref(),
            self.registration.nick(),
            self.isupport.value(),
            self.no_implicit_names(),
            MetadataContext {
                max_keys: state::MAX_METADATA_KEYS_PER_TARGET,
                subscription_batch: false,
            },
            None,
            &mut self.outbox,
        );
        let generated = self.outbox.events.split_off(event_count);
        self.outbox.events.extend(
            generated
                .into_iter()
                .filter(|event| matches!(event, Event::StateLimitReached { .. })),
        );
        if before != self.state {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    fn no_implicit_names(&self) -> bool {
        self.caps
            .enabled()
            .keys()
            .any(|name| papo_proto::canonical(name) == "no-implicit-names")
    }

    fn maybe_subscribe_metadata(&mut self, now: Instant) {
        if self.metadata_subscriptions_sent || self.config.metadata_subscriptions.is_empty() {
            return;
        }
        let Some(limits) = self.metadata_limits() else {
            return;
        };
        let registered = self.registration.is_registered();
        if !registered && !limits.before_connect {
            return;
        }
        self.metadata_subscriptions_sent = true;
        let result = self.metadata(
            now,
            MetadataCommand::Sub {
                keys: self.config.metadata_subscriptions.clone(),
            },
        );
        if !registered
            && result.is_ok()
            && let Some(Transmission::Line(line)) = self.outbox.transmits.pop_back()
        {
            self.outbox.immediate.push_back(line);
        }
    }

    fn close_for_long_line(&mut self) {
        self.close(
            CloseKind::LineTooLong,
            "incoming IRC line exceeds 8703 bytes".to_owned(),
        );
    }

    fn sync_monitored_state(&mut self) {
        self.state
            .sync_monitored(&self.monitor, self.isupport.value());
    }

    fn close(&mut self, kind: CloseKind, reason: String) {
        self.outbox.close(kind, reason);
        self.apply_close_request();
    }

    fn apply_close_request(&mut self) {
        if self.outbox.close_requested {
            self.framer.close();
            self.keepalive.handle_disconnected();
        }
    }
}

fn bypasses_rate_limit(line: &[u8]) -> bool {
    let mut fields = line.split(|byte| *byte == b' ');
    let first = fields.next().unwrap_or_default();
    let command = if first.starts_with(b"@") {
        fields.next().unwrap_or_default()
    } else {
        first
    };
    // Ergo exempts handshake traffic, and a delayed PONG risks disconnection.
    // These commands can safely overtake queued application traffic; QUIT cannot.
    [
        b"WEBIRC".as_slice(),
        b"PASS",
        b"NICK",
        b"USER",
        b"CAP",
        b"AUTHENTICATE",
        b"REGISTER",
        b"VERIFY",
        b"PONG",
    ]
    .iter()
    .any(|exempt| command.eq_ignore_ascii_case(exempt))
}

fn requested_message_feature(message: &MessageRef<'_>) -> Option<Feature> {
    if message.command_is("REDACT") {
        return Some(Feature::Redaction);
    }
    if message.tag("+draft/react").is_some() || message.tag("+draft/unreact").is_some() {
        return Some(Feature::Reactions);
    }
    message.tag("+typing").map(|_| Feature::Typing)
}

fn parse_whois_channels(lines: &[Message], isupport: &papo_proto::Isupport) -> Vec<WhoisChannel> {
    let symbols: Vec<_> = isupport
        .prefix()
        .into_iter()
        .map(|prefix| prefix.symbol)
        .collect();
    lines
        .iter()
        .filter_map(|line| {
            let message = line.as_ref();
            (message.command() == Command::Numeric(Numeric::RPL_WHOISCHANNELS))
                .then(|| message.param(2))
                .flatten()
        })
        .flat_map(str::split_ascii_whitespace)
        .filter_map(|channel| {
            let prefix_bytes = channel
                .chars()
                .take_while(|symbol| symbols.contains(symbol))
                .map(char::len_utf8)
                .sum();
            let (prefixes, name) = channel.split_at(prefix_bytes);
            (!name.is_empty()).then(|| WhoisChannel {
                name: name.to_owned(),
                prefixes: prefixes.chars().collect(),
            })
        })
        .collect()
}

fn encode_label(mut value: u64) -> String {
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut buffer = [0_u8; 11];
    let mut index = buffer.len();
    loop {
        index -= 1;
        buffer[index] = ALPHABET[(value % 62) as usize];
        value /= 62;
        if value == 0 {
            return String::from_utf8(buffer[index..].to_vec()).expect("labels are ASCII");
        }
    }
}

fn sent_info(message: &MessageRef<'_>) -> Option<SentInfo> {
    Some(SentInfo {
        msgid: message.tag("msgid")?.into_owned(),
        time: message.tag("time").map(std::borrow::Cow::into_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::{CloseKind, Event, Outbox};

    #[test]
    fn batch_queue_rejects_empty_and_closed_transmissions() {
        let mut outbox = Outbox::default();
        outbox.transmit_batch(Vec::new());
        assert!(outbox.transmits.is_empty());

        outbox.close_requested = true;
        outbox.transmit_batch(vec![b"BATCH +papo draft/multiline #papo\r\n".to_vec()]);
        assert!(outbox.transmits.is_empty());
    }

    #[test]
    fn generic_protocol_closures_retain_the_other_kind() {
        let mut outbox = Outbox::default();
        outbox.close(CloseKind::Other, "protocol shutdown".to_owned());
        assert!(matches!(
            outbox.events.pop_front(),
            Some(Event::Close {
                kind: CloseKind::Other,
                ..
            })
        ));
    }

    #[test]
    fn invalid_internal_messages_close_without_panicking() {
        for immediate in [false, true] {
            let mut outbox = Outbox::default();
            if immediate {
                outbox.transmit_immediate("NOTICE", ["alice", "bad\rtext"]);
            } else {
                outbox.transmit("NOTICE", ["alice", "bad\rtext"]);
            }
            assert!(matches!(
                outbox.events.pop_front(),
                Some(Event::Close {
                    kind: CloseKind::Other,
                    ..
                })
            ));
            assert!(outbox.transmits.is_empty());
            assert!(outbox.immediate.is_empty());
        }
    }
}
