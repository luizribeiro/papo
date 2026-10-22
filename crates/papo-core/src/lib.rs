//! A Sans-I/O IRC session state machine: protocol behavior, time, and randomness are driven by
//! explicit inputs. Most users want the `papo` crate; use `papo-core` directly to embed its
//! deterministic protocol engine behind a custom I/O layer.
//!
//! ## Capability implementation map
//!
//! <!-- caps:start -->
//! | Capability | Aliases | Requested | Features | Code |
//! |---|---|---|---|---|
//! | `cap-notify` | — | default | — | `caps` |
//! | `message-tags` | — | default | client-only message tags, replies, reactions, typing notifications | `features`, `replies` |
//! | `server-time` | — | default | — | `state::read_marker` |
//! | `batch` | — | default | — | `batch` |
//! | `draft/event-playback` | — | default | — | `batch` |
//! | `echo-message` | — | default | — | `outgoing`, `echo` |
//! | `account-tag` | — | default | account tracking | `state::presence` |
//! | `extended-join` | — | default | — | `state::membership` |
//! | `multi-prefix` | — | default | — | `state::membership` |
//! | `userhost-in-names` | — | default | — | `state::membership` |
//! | `chghost` | — | default | host tracking | `state::presence` |
//! | `setname` | — | default | real-name changes | `state::presence` |
//! | `away-notify` | — | default | away tracking | `state::presence` |
//! | `account-notify` | — | default | account tracking | `state::presence` |
//! | `extended-monitor` | — | default | — | `monitor` |
//! | `invite-notify` | — | default | invite tracking | `replies` |
//! | `standard-replies` | — | default | — | `refusal`, `replies` |
//! | `labeled-response` | — | default | labeled-response | `outgoing`, `labels` |
//! | `draft/multiline` | — | default | multiline messages | `outgoing`, `batch` |
//! | `draft/oper-tag` | — | default | — | `replies` |
//! | `draft/metadata-3` | draft/metadata-2 | default | metadata | `metadata`, `state::metadata` |
//! | `metadata-notify` | — | fallback | — | `metadata`, `state::metadata` |
//! | `sasl` | — | automatic | — | `sasl` |
//! | `sts` | — | never | — | `sts` |
//! | `ergo.chat/nope` | — | never | — | `caps` |
//! | `soju.im/account-required` | — | never | — | `caps` |
//! | `draft/account-registration` | account-registration | default | account creation | `account_registration` |
//! | `soju.im/bouncer-networks` | — | opt-in | bouncer networks | `bouncer` |
//! | `soju.im/bouncer-networks-notify` | — | opt-in | — | `bouncer` |
//! | `draft/chathistory` | chathistory | default | chathistory | `history`, `batch` |
//! | `draft/read-marker` | read-marker | default | read-marker | `history`, `state::read_marker` |
//! | `draft/persistence` | — | opt-in | persistence | `persistence`, `state::persistence` |
//! | `draft/channel-rename` | — | default | channel rename | `channel_rename`, `state::rename` |
//! | `draft/relaymsg` | — | opt-in | relaymsg | `relaymsg` |
//! | `draft/message-redaction` | — | default | draft/message-redaction | `outgoing`, `replies` |
//! | `draft/webpush` | soju.im/webpush | opt-in | webpush | `webpush` |
//! | `draft/pre-away` | — | default | — | `state` |
//! | `no-implicit-names` | draft/no-implicit-names | default | — | `state::membership` |
//! | `draft/whoami` | — | default | — | `state` |
//! | `draft/extended-isupport` | — | default | — | `isupport` |
//! <!-- caps:end -->

#![cfg_attr(docsrs, feature(doc_cfg))]

mod account_registration;
mod batch;
#[cfg(feature = "unstable")]
#[path = "unstable/bouncer.rs"]
mod bouncer;
pub mod cap;
mod caps;
mod channel_rename;
mod conversation;
mod echo;
mod features;
mod framing;
mod history;
mod isupport;
mod keepalive;
mod labels;
mod metadata;
mod monitor;
mod name;
mod nick_regain;
mod outgoing;
#[cfg(feature = "unstable")]
#[path = "unstable/persistence.rs"]
mod persistence;
mod ratelimit;
mod refusal;
#[cfg(feature = "unstable")]
#[path = "unstable/relaymsg.rs"]
mod relaymsg;
mod replies;
mod sasl;
mod source;
mod state;
pub mod sts;
mod typing;
#[cfg(feature = "unstable")]
#[path = "unstable/webpush.rs"]
mod webpush;
mod welcome;

#[cfg(feature = "unstable")]
include!("unstable/persistence_types.rs");

pub use papo_proto::{Secret, WireLine};
pub use source::Source;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Duration,
};
use web_time::Instant;

use papo_proto::{
    AccountName, BuildError, Casemapping, ChannelName, Command, Ctcp, Line, LineRef, MAX_LINE_LEN,
    MAX_TAG_SECTION_LEN, Mask, Nick, Numeric, Source as ProtoSource,
    StandardReply as StandardReplyView, StandardReplyKind, escape_tag_value,
};

#[cfg(feature = "unstable")]
pub use self::bouncer::{BouncerNetwork, NetworkConfig, NetworkId, NetworkState};
use account_registration::{AccountRegistration, PendingRegistration, RegistrationPolicy};
use batch::{BatchAction, BatchEvent, Batches, DispatchContext};
use caps::Caps;
pub use features::{Feature, Unsupported, UnsupportedReason};
use framing::Framer;
pub use history::{HistoryQuery, LastSeen, MsgRef, ReadMarker};
use isupport::IsupportState;
use keepalive::Keepalive;
use labels::encode_label;
pub use metadata::{MetadataCommand, MetadataLimits, MetadataValue, Visibility};
use monitor::Monitor;
pub(crate) use name::{NameKey, names_equal};
use nick_regain::NickRegain;
pub use outgoing::TextKind;
pub use papo_proto::ModeAction;
use ratelimit::RateLimiter;
pub use ratelimit::{InvalidRateLimit, RateLimit};
pub use refusal::Refusal;
use sasl::SaslSession;
pub use sasl::{Entropy, Sasl};
use state::MetadataContext;
pub use state::{Account, Away, Channel, Member, Rank, State, User};
use sts::ConnectionSecurity;
#[doc(hidden)]
pub use typing::TypingTable;
pub use typing::{Typer, TypingChanged, TypingReason};
use welcome::WelcomeState;

const MAX_INCOMING_LINE_LEN: usize = MAX_TAG_SECTION_LEN + MAX_LINE_LEN;
const DEFAULT_WELCOME_TIMEOUT: Duration = Duration::from_secs(60);
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
    /// Write-completion identifier when no server response is expected.
    pub transmit: Option<TransmitToken>,
    /// Number of server confirmations expected for the request.
    pub confirmations: usize,
    /// Whether the text was divided into multiple message lines.
    pub split: bool,
}

/// Opaque identifier for one user-originated transmission.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TransmitToken(u64);

/// One line ready for the transport and its optional completion identifier.
#[doc(hidden)]
#[derive(Clone, Eq, PartialEq)]
pub struct Transmit {
    line: WireLine,
    token: Option<TransmitToken>,
}

impl std::fmt::Debug for Transmit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Transmit")
            .field("line", &self.line)
            .field("token", &self.token)
            .finish()
    }
}

impl Transmit {
    /// Consumes the transmission into its wire line and completion identifier.
    #[must_use]
    pub fn into_parts(self) -> (WireLine, Option<TransmitToken>) {
        (self.line, self.token)
    }
}
const CTCP_REPLY_INTERVAL: Duration = Duration::from_secs(2);

pub use cap::Cap;
/// Capabilities requested by a default session when the server advertises them.
pub use cap::DEFAULTS as DEFAULT_WANTED_CAPS;

/// Enabled `IRCv3` capability names and their advertised values.
pub type Capabilities = BTreeMap<cap::Cap, Option<String>>;

/// The IRC conversation to which a chat line belongs.
///
/// A conversation never includes a `STATUSMSG` prefix from its wire address.
///
/// # Status-prefixed lines
///
/// A line addressed to `@#ops` belongs to the `#ops` conversation and records `@` separately in
/// its `status` field. Only channel operators received that line. Answer through the line's
/// `reply`, `react`, or `return_address()` path. Never construct a send target from a
/// `Conversation` alone, because that loses the prefix and widens the audience.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Conversation {
    /// Messages addressed to a channel.
    Channel(ChannelName),
    /// Messages exchanged with the named other party.
    ///
    /// The session's own nickname represents a notes-to-self conversation.
    Private(Nick),
    /// Messages sent by the server, including messages addressed to `*`.
    Server,
    /// Operator broadcasts addressed to a server mask.
    ///
    /// Ergo rewrites these messages to each recipient's nickname, so they appear as private
    /// conversations when received from Ergo.
    Broadcast(Mask),
}

impl Conversation {
    fn as_str(&self) -> &str {
        match self {
            Self::Channel(channel) => channel,
            Self::Private(nick) => nick,
            Self::Server => "*",
            Self::Broadcast(mask) => mask,
        }
    }
}

impl std::fmt::Display for Conversation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq<str> for Conversation {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::Channel(channel) => channel == other,
            Self::Private(nick) => nick == other,
            Self::Server => other == "*",
            Self::Broadcast(mask) => mask == other,
        }
    }
}

impl PartialEq<&str> for Conversation {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

/// A user or channel acted on by an IRC event.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Subject {
    /// A channel acted on by the event.
    Channel(ChannelName),
    /// A user acted on by the event.
    User(Nick),
}

/// Whether an event's actor is this session's user.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Authorship {
    /// The actor is this session's user.
    Own,
    /// The actor is another user or a server.
    Other,
    /// The protocol did not prove who the actor is relative to this session.
    Unknown,
}

/// Classifies the IRC conversation and `STATUSMSG` prefixes addressed by a line.
#[doc(hidden)]
pub fn classify_conversation(
    line: &LineRef<'_>,
    own_nick: &str,
    isupport: &papo_proto::Isupport,
) -> Result<(Option<Vec<char>>, Conversation), papo_proto::InvalidIdentifier> {
    conversation::classify_line(line, own_nick, isupport)
}

/// Automatic replies to CTCP queries.
///
/// | Query | Answered by default | Configuration |
/// |---|---|---|
/// | `PING` | Yes, by echoing its parameters | [`ping(false)`](Self::ping) disables it |
/// | `VERSION` | Yes, with `papo <version>` | [`version(Some(...))`](Self::version) changes it; [`version(None)`](Self::version) disables it |
/// | Anything else | No | Handle the CTCP event and call `Client::ctcp_reply` |
///
/// Automatic replies are sent only for live queries addressed privately to this session's nick.
/// Queries replayed in batches, sent to channels or status-prefixed channel targets, or copied
/// from another session of the same user are still reported as events but are not answered.
/// Replies are limited to one per sender every two seconds and by the overall
/// [`rate`](Self::rate).
///
/// The default answers `PING` and `VERSION`:
///
/// ```
/// # use papo_core::CtcpReplies;
/// let replies = CtcpReplies::default();
/// # _ = replies;
/// ```
///
/// A custom application version can replace the default text:
///
/// ```
/// # use papo_core::CtcpReplies;
/// let replies = CtcpReplies::default().version(Some("ferrobot 2.3"));
/// # _ = replies;
/// ```
///
/// Either reply can be selected from the disabled starting point:
///
/// ```
/// # use papo_core::CtcpReplies;
/// let ping_only = CtcpReplies::off().ping(true);
/// let version_only = CtcpReplies::off().version(Some("ferrobot 2.3"));
/// # _ = (ping_only, version_only);
/// ```
///
/// Use [`CtcpReplies::off`] when the application will answer CTCP events itself or should remain
/// silent.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CtcpReplies {
    /// VERSION reply, or `None` to disable it.
    version: Option<String>,
    /// Whether PING parameters are echoed.
    ping: bool,
    /// Overall reply budget shared across all senders.
    rate: RateLimit,
}

/// Away state to announce when connecting.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AwayOnConnect {
    /// Announce this message. Without `draft/pre-away`, it is sent after connecting.
    Message(String),
    /// Announce hidden away state with `AWAY *`. Without `draft/pre-away`, it is skipped.
    Hidden,
}

/// Controls whether a channel's members are refreshed with `WHO` after joining.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum WhoOnJoin {
    /// Refresh members after every channel join.
    #[default]
    Always,
    /// Do not refresh members automatically.
    Never,
    /// Refresh members only when fewer than this many were reported by `NAMES`.
    BelowMembers(usize),
}

impl Default for CtcpReplies {
    fn default() -> Self {
        Self {
            version: Some(format!("papo {}", env!("CARGO_PKG_VERSION"))),
            ping: true,
            rate: RateLimit::new(2, Duration::from_secs(2))
                .expect("the default CTCP reply rate is valid"),
        }
    }
}

impl CtcpReplies {
    /// Disables every automatic CTCP reply.
    #[must_use]
    pub fn off() -> Self {
        Self {
            version: None,
            ping: false,
            rate: Self::default().rate,
        }
    }

    /// Sets the VERSION reply, or disables it with `None`.
    ///
    /// Prefer an application name and version without operating-system or build details, which
    /// make clients easier to fingerprint.
    #[must_use]
    pub fn version(mut self, text: Option<&str>) -> Self {
        self.version = text.map(str::to_owned);
        self
    }

    /// Enables or disables PING replies.
    #[must_use]
    pub const fn ping(mut self, enabled: bool) -> Self {
        self.ping = enabled;
        self
    }

    /// Sets the overall reply budget shared across all senders.
    #[must_use]
    pub const fn rate(mut self, rate: RateLimit) -> Self {
        self.rate = rate;
        self
    }
}

/// Builds the private `NOTICE` used to answer a CTCP query.
#[doc(hidden)]
#[must_use]
pub fn __ctcp_reply(actor: &Source, command: &str, text: &str) -> Option<Line> {
    let nick = actor.nick()?;
    let body = Ctcp::build(command, (!text.is_empty()).then_some(text))?;
    Some(Line::notice(nick.as_ref(), body))
}

/// Connection state captured when a session becomes ready for use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Welcome {
    /// The nickname assigned by the server.
    pub nick: Nick,
    /// The human-readable text from the server's `001` welcome numeric.
    pub text: String,
    /// The enabled capabilities and their advertised values.
    pub caps: Capabilities,
    /// The accumulated server feature advertisement.
    pub isupport: papo_proto::Isupport,
}

/// Policy for decoding incoming IRC lines.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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
    /// Client IP address.
    pub client_ip: std::net::IpAddr,
    /// Client hostname, or `None` to send the client's IP address as its hostname.
    pub client_hostname: Option<String>,
    /// Ordered `name[=value]` options appended to the command.
    pub options: Vec<(String, Option<String>)>,
}

/// Configuration for an IRC session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Config {
    /// The policy used to decode incoming lines.
    pub decoding: Decoding,
    /// Name casemapping remembered before a server advertises its rules.
    pub casemapping: Casemapping,
    /// The nickname requested while connecting.
    pub nick: String,
    /// The username sent while connecting.
    pub username: String,
    /// The real name sent while connecting.
    pub realname: String,
    /// The optional server password sent while connecting.
    pub password: Option<Secret>,
    /// Gateway identity sent before connecting begins.
    pub webirc: Option<WebIrc>,
    /// The maximum time to wait for the welcome numeric.
    pub welcome_timeout: Duration,
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
    /// Optional extension features explicitly enabled by the application.
    #[cfg(feature = "unstable")]
    pub enabled_features: BTreeSet<Feature>,
    /// Alternative nicknames to try when the requested nickname collides while connecting.
    pub alt_nicks: Vec<String>,
    /// Whether to reclaim the configured nickname after a connection-time collision.
    pub regain_nick: bool,
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
    /// Away state announced during or immediately after connecting.
    pub away_on_connect: Option<AwayOnConnect>,
    /// Policy for refreshing channel members after joining.
    pub who_on_join: WhoOnJoin,
    /// Metadata keys subscribed on every connection.
    pub metadata_subscriptions: Vec<String>,
    allow_insecure_auth: bool,
    /// Upstream bouncer network selected before the connection completes.
    #[cfg(feature = "unstable")]
    pub bouncer_network: Option<NetworkId>,
    registration_on_connect: Option<PendingRegistration>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            decoding: Decoding::default(),
            casemapping: Casemapping::default(),
            nick: "papo".to_owned(),
            username: "papo".to_owned(),
            realname: "papo".to_owned(),
            password: None,
            webirc: None,
            welcome_timeout: DEFAULT_WELCOME_TIMEOUT,
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
            #[cfg(feature = "unstable")]
            enabled_features: BTreeSet::new(),
            alt_nicks: Vec::new(),
            regain_nick: false,
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
            who_on_join: WhoOnJoin::default(),
            metadata_subscriptions: Vec::new(),
            allow_insecure_auth: false,
            #[cfg(feature = "unstable")]
            bouncer_network: None,
            registration_on_connect: None,
        }
    }
}

impl Config {
    fn selected_bouncer_network(&self) -> Option<&str> {
        #[cfg(feature = "unstable")]
        return self.bouncer_network.as_ref().map(AsRef::as_ref);
        #[cfg(not(feature = "unstable"))]
        None
    }

    /// Enables an unstable extension feature before connecting.
    #[cfg(feature = "unstable")]
    #[must_use]
    pub fn enable(mut self, feature: Feature) -> Self {
        self.enabled_features.insert(feature);
        for info in cap::ALL {
            if let cap::Request::OptIn(name, owner) = info.request
                && owner == feature
                && !self.wanted_caps.iter().any(|wanted| wanted == name)
            {
                self.wanted_caps.push(name.to_owned());
            }
        }
        self
    }

    /// Sets the incoming line decoding policy.
    #[must_use]
    pub const fn decoding(mut self, decoding: Decoding) -> Self {
        self.decoding = decoding;
        self
    }

    /// Seeds the last-known server casemapping while disconnected.
    #[must_use]
    pub const fn casemapping(mut self, casemapping: Casemapping) -> Self {
        self.casemapping = casemapping;
        self
    }

    /// Sets the nickname requested while connecting.
    #[must_use]
    pub fn nick(mut self, nick: impl Into<String>) -> Self {
        self.nick = nick.into();
        self
    }

    /// Sets the username sent while connecting.
    #[must_use]
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = username.into();
        self
    }

    /// Sets the real name sent while connecting.
    #[must_use]
    pub fn realname(mut self, realname: impl Into<String>) -> Self {
        self.realname = realname.into();
        self
    }

    /// Sets the optional server password sent while connecting.
    #[must_use]
    pub fn password(mut self, password: impl Into<Secret>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the gateway identity sent with `WEBIRC` before connecting.
    #[must_use]
    pub fn webirc(mut self, webirc: WebIrc) -> Self {
        self.webirc = Some(webirc);
        self
    }

    /// Validates the registration commands derived from this configuration.
    #[doc(hidden)]
    pub fn validate_registration(&self) -> Result<(), BuildError> {
        Line::new("NICK").param(&self.nick).encode()?;
        Line::new("USER")
            .param(&self.username)
            .param("0")
            .param("*")
            .param(&self.realname)
            .encode()?;
        if let Some(password) = &self.password
            && password.expose().is_some()
        {
            Line::new("PASS").param(password).encode()?;
        }
        for nick in &self.alt_nicks {
            Line::new("NICK").param(nick).encode()?;
        }
        if let Some(AwayOnConnect::Message(message)) = &self.away_on_connect {
            Line::new("AWAY").param(message).encode()?;
        }
        if let Some(webirc) = &self.webirc
            && webirc.password.expose().is_some()
        {
            webirc_line(webirc)?;
        }
        Ok(())
    }

    /// Stages account creation through the account-registration extension.
    #[must_use]
    pub fn create_account_on_connect(
        mut self,
        account: Option<impl Into<String>>,
        email: Option<impl Into<String>>,
        password: impl Into<Secret>,
    ) -> Self {
        self.registration_on_connect = Some(PendingRegistration {
            request: None,
            account: account.map(Into::into),
            email: email.map(Into::into),
            password: password.into(),
        });
        self
    }

    /// Allows credentials to be sent over a plaintext transport.
    ///
    /// This is intended for controlled test fixtures. Credentials are refused on plaintext
    /// connections by default.
    #[must_use]
    pub const fn allow_insecure_auth(mut self, allow: bool) -> Self {
        self.allow_insecure_auth = allow;
        self
    }

    /// Sets the maximum time to wait for the welcome numeric.
    #[must_use]
    pub const fn welcome_timeout(mut self, timeout: Duration) -> Self {
        self.welcome_timeout = timeout;
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
            .any(|disabled| cap::canonical(disabled) == cap::canonical(name))
    }

    /// Sets the alternative nicknames tried while connecting.
    #[must_use]
    pub fn alt_nicks<I, S>(mut self, nicks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.alt_nicks = nicks.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the SASL mechanisms to try while connecting.
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
    /// sent after connecting completes and hidden away state is skipped.
    #[must_use]
    pub fn away_on_connect(mut self, away: Option<AwayOnConnect>) -> Self {
        self.away_on_connect = away;
        self
    }

    /// Sets when joined-channel members are refreshed with `WHO`.
    #[must_use]
    pub fn who_on_join(mut self, policy: WhoOnJoin) -> Self {
        self.who_on_join = policy;
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

fn webirc_line(webirc: &WebIrc) -> Result<WireLine, BuildError> {
    let client_ip = webirc.client_ip.to_string();
    let ip = if client_ip.starts_with(':') {
        format!("0{client_ip}")
    } else {
        client_ip
    };
    let hostname = webirc.client_hostname.as_deref().unwrap_or(&ip);
    let mut line = Line::new("WEBIRC")
        .param(&webirc.password)
        .param(&webirc.gateway)
        .param(hostname)
        .param(&ip);
    let options = webirc
        .options
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{name}={}", escape_tag_value(value)),
            None => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    if !options.is_empty() {
        line = line.param(options);
    }
    line.encode()
}

/// One operation from an IRC mode string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeChange {
    /// The mode letter.
    pub mode: char,
    /// Whether the mode was added or removed.
    pub action: ModeAction,
    /// The mode argument classified using the server's advertised rules.
    pub arg: Option<ModeArg>,
}

/// A channel mode whose entries are reported as a list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ListMode {
    /// Channel bans (`+b`).
    Ban,
    /// Ban exceptions (`+e`).
    Except,
    /// Invite exceptions (`+I`).
    Invex,
    /// Quiet masks (`+q`).
    Quiet,
}

impl ListMode {
    const fn numerics(self) -> (u16, u16) {
        match self {
            Self::Ban => (367, 368),
            Self::Except => (348, 349),
            Self::Invex => (346, 347),
            Self::Quiet => (728, 729),
        }
    }
}

/// One entry reported for a channel list mode.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeListEntry {
    /// Mask stored by the server.
    pub mask: Mask,
    /// Actor that set the entry, when reported.
    pub actor: Option<Source>,
    /// Time at which the entry was set, when reported and valid.
    pub set_at: Option<std::time::SystemTime>,
}

/// A mode argument classified using the server's advertised rules.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModeArg {
    /// A nickname for a channel-membership mode advertised in `PREFIX`.
    Nick(Nick),
    /// A mask for a list mode advertised in the first `CHANMODES` group.
    Mask(Mask),
    /// A value for any other mode that takes an argument.
    Value(String),
}

/// The stored value of a channel mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeValue<'a> {
    /// The mode is set without an argument.
    Set,
    /// The mode is set with an argument.
    Value(&'a str),
}

impl<'a> ModeValue<'a> {
    /// Returns the mode argument, if it has one.
    #[must_use]
    pub const fn value(self) -> Option<&'a str> {
        match self {
            Self::Set => None,
            Self::Value(value) => Some(value),
        }
    }
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
    #[non_exhaustive]
    Kick {
        /// Channel being changed.
        channel: String,
        /// Nickname being removed.
        nick: String,
    },
    /// A channel invitation ending at `RPL_INVITING`.
    #[non_exhaustive]
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
    /// A channel mode list ending at its mode-specific numeric.
    #[non_exhaustive]
    ListMode {
        /// Channel whose list was requested.
        channel: String,
        /// List mode that was requested.
        mode: ListMode,
    },
    /// A read-marker query ending at the matching `MARKREAD` reply.
    ReadMarker(String),
}

impl ResponseMatcher {
    #[doc(hidden)]
    #[must_use]
    pub fn __kick(channel: String, nick: String) -> Self {
        Self::Kick { channel, nick }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __invite(channel: String, nick: String) -> Self {
        Self::Invite { channel, nick }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __list_mode(channel: String, mode: ListMode) -> Self {
        Self::ListMode { channel, mode }
    }

    pub(crate) fn is_list_row(&self, message: &LineRef<'_>, mapping: Casemapping) -> bool {
        let Self::ListMode { channel, mode } = self else {
            return false;
        };
        let Command::Numeric(numeric) = message.command() else {
            return false;
        };
        numeric.code() == mode.numerics().0
            && Self::target_is(message, 1, channel, mapping)
            && (*mode != ListMode::Quiet || message.param(2) == Some("q"))
    }

    fn standard_failure(message: &LineRef<'_>, command: &str) -> bool {
        StandardReplyView::parse(message).is_some_and(|reply| {
            reply.kind == StandardReplyKind::Fail && reply.command.eq_ignore_ascii_case(command)
        })
    }

    fn target_is(message: &LineRef<'_>, index: usize, target: &str, mapping: Casemapping) -> bool {
        message
            .param(index)
            .is_some_and(|value| names_equal(mapping, value, target))
    }

    fn numeric_names_command(message: &LineRef<'_>, numeric: Numeric, command: &str) -> bool {
        (numeric.is_error() || numeric == Numeric::RPL_TRYAGAIN)
            && message
                .param(1)
                .is_some_and(|value| value.eq_ignore_ascii_case(command))
    }

    fn own_channel_departure(
        message: &LineRef<'_>,
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
                    .is_some_and(|nick| names_equal(mapping, nick, own_nick)))
    }

    #[allow(clippy::too_many_lines)]
    fn matches(
        &self,
        message: &LineRef<'_>,
        mapping: Casemapping,
        own_nick: &str,
        state: &State,
    ) -> Option<bool> {
        let numeric = match message.command() {
            Command::Numeric(numeric) => Some(numeric),
            _ => None,
        };
        let own_source = matches!(message.source(), Some(ProtoSource::User { nick, .. }) if names_equal(mapping, nick, own_nick));
        let current_own_source = matches!(message.source(), Some(ProtoSource::User { nick, .. }) if names_equal(mapping, nick, state.nick()));
        match self {
            Self::Whois(nick) => {
                if Self::standard_failure(message, "WHOIS")
                    || numeric.is_some_and(|numeric| {
                        Self::numeric_names_command(message, numeric, "WHOIS")
                    })
                {
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
                        | Numeric::ERR_NOSUCHNICK
                );
                (relevant && Self::target_is(message, 1, nick, mapping)).then_some(matches!(
                    numeric,
                    Numeric::RPL_ENDOFWHOIS | Numeric::ERR_NOSUCHNICK
                ))
            }
            Self::Who(mask) => {
                if Self::standard_failure(message, "WHO")
                    || numeric
                        .is_some_and(|numeric| Self::numeric_names_command(message, numeric, "WHO"))
                {
                    return Some(true);
                }
                numeric.and_then(|numeric| {
                    let row = matches!(numeric, Numeric::RPL_WHOREPLY | Numeric::RPL_WHOSPCRPL);
                    let end = numeric == Numeric::RPL_ENDOFWHO
                        && Self::target_is(message, 1, mask, mapping);
                    (row || end).then_some(end)
                })
            }
            Self::Join(channel) => {
                if Self::standard_failure(message, "JOIN")
                    || numeric.is_some_and(|numeric| {
                        Self::numeric_names_command(message, numeric, "JOIN")
                    })
                {
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
                    let refusal = matches!(
                        numeric.code(),
                        403 | 405 | 437 | 471 | 473 | 474 | 475 | 476 | 477 | 489
                    );
                    (named(1) && (numeric == Numeric::RPL_ENDOFNAMES || refusal)).then_some(true)
                })
            }
            Self::Part(channel) => ((message.command() == Command::Part
                && own_source
                && Self::target_is(message, 0, channel, mapping))
                || (numeric.is_some_and(|numeric| matches!(numeric.code(), 403 | 442))
                    && Self::target_is(message, 1, channel, mapping))
                || numeric
                    .is_some_and(|numeric| Self::numeric_names_command(message, numeric, "PART"))
                || Self::standard_failure(message, "PART"))
            .then_some(true),
            Self::Mode(target) => ((message.command() == Command::Mode
                && Self::target_is(message, 0, target, mapping))
                || numeric.is_some_and(|numeric| {
                    matches!(numeric.code(), 472 | 501 | 502)
                        || numeric.is_error() && Self::target_is(message, 1, target, mapping)
                        || Self::numeric_names_command(message, numeric, "MODE")
                })
                || Self::standard_failure(message, "MODE"))
            .then_some(true),
            Self::Topic(channel) => {
                let named = |index| Self::target_is(message, index, channel, mapping);
                ((message.command() == Command::Topic && named(0))
                    || numeric.is_some_and(|numeric| {
                        named(1)
                            && (matches!(numeric, Numeric::RPL_NOTOPIC | Numeric::RPL_TOPIC)
                                || matches!(numeric.code(), 403 | 442 | 482))
                            || Self::numeric_names_command(message, numeric, "TOPIC")
                    })
                    || Self::standard_failure(message, "TOPIC"))
                .then_some(true)
            }
            Self::Kick { channel, nick } => {
                let named = |index, expected| Self::target_is(message, index, expected, mapping);
                ((message.command() == Command::Kick && named(0, channel) && named(1, nick))
                    || numeric.is_some_and(|numeric| match numeric.code() {
                        401 => named(1, nick),
                        403 | 442 | 482 => named(1, channel),
                        441 | 484 => named(1, nick) && named(2, channel),
                        _ => Self::numeric_names_command(message, numeric, "KICK"),
                    })
                    || Self::standard_failure(message, "KICK"))
                .then_some(true)
            }
            Self::Invite { channel, nick } => {
                let named = |index, expected| Self::target_is(message, index, expected, mapping);
                (numeric.is_some_and(|numeric| {
                    (numeric == Numeric::RPL_INVITING && named(1, nick) && named(2, channel))
                        || match numeric.code() {
                            401 => named(1, nick),
                            403 | 442 | 482 => named(1, channel),
                            443 => named(1, nick) && named(2, channel),
                            _ => Self::numeric_names_command(message, numeric, "INVITE"),
                        }
                }) || Self::standard_failure(message, "INVITE"))
                .then_some(true)
            }
            Self::Nick(nick) => ((message.command() == Command::Nick
                && own_source
                && Self::target_is(message, 0, nick, mapping))
                || numeric.is_some_and(|numeric| {
                    numeric == Numeric::ERR_NONICKNAMEGIVEN
                        || numeric.is_error() && Self::target_is(message, 1, nick, mapping)
                        || Self::numeric_names_command(message, numeric, "NICK")
                })
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
                || numeric == Some(Numeric::ERR_NOORIGIN)
                || numeric
                    .is_some_and(|numeric| Self::numeric_names_command(message, numeric, "PING"))
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
            Self::ListMode { channel, mode } => {
                if Self::standard_failure(message, "MODE") {
                    return Some(true);
                }
                let Command::Numeric(numeric) = message.command() else {
                    return None;
                };
                let named = Self::target_is(message, 1, channel, mapping);
                let (row, end) = mode.numerics();
                let list_reply = named
                    && matches!(numeric.code(), code if code == row || code == end)
                    && (*mode != ListMode::Quiet || message.param(2) == Some("q"));
                (list_reply
                    || numeric.is_error() && named
                    || Self::numeric_names_command(message, numeric, "MODE"))
                .then_some(numeric.code() == end || numeric.is_error())
            }
            Self::ReadMarker(target) => ((message.command() == Command::Markread
                && Self::target_is(message, 0, target, mapping))
                || Self::standard_failure(message, "MARKREAD"))
            .then_some(true),
        }
    }
}

#[derive(Debug)]
struct UnlabelledRequest {
    id: RequestId,
    started: Option<Instant>,
    matcher: ResponseMatcher,
    fence: Option<String>,
    deferred: Option<WireLine>,
    lines: Vec<Line>,
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
    pub lines: Vec<Line>,
    /// A typed history page nested in this response, when present.
    pub history: Option<HistoryPage>,
    /// Targets nested in a completed `CHATHISTORY TARGETS` response.
    pub history_targets: Option<Vec<(Conversation, std::time::SystemTime)>>,
    /// Channels parsed from `RPL_WHOISCHANNELS` using the current `PREFIX` token.
    pub whois_channels: Vec<WhoisChannel>,
    /// Parsed channel list-mode entries, when this completed such a request.
    pub mode_list: Option<Vec<ModeListEntry>>,
    /// State bound that ended the request before its protocol terminator.
    pub state_limit: Option<StateLimit>,
    sent: Option<SentInfo>,
    enabled_features: BTreeSet<Feature>,
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
                    #[cfg(feature = "unstable")]
                    refusal: Refusal::from_standard_enabled(
                        reply.command,
                        reply.code,
                        &self.enabled_features,
                    ),
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
    pub server_time: Option<String>,
}

/// Error reported by an IRC server while processing a request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ServerError {
    /// An `IRCv3` `FAIL` standard reply.
    #[non_exhaustive]
    Standard {
        /// Command associated with the failure.
        command: String,
        /// Case-normalized failure code.
        code: String,
        /// Additional diagnostic parameters.
        context: Vec<String>,
        /// Human-readable failure description.
        description: String,
        /// Classification recorded using the negotiated unstable features.
        #[cfg(feature = "unstable")]
        #[doc(hidden)]
        refusal: Refusal,
    },
    /// A classic error numeric.
    #[non_exhaustive]
    Numeric {
        /// Numeric error code.
        numeric: Numeric,
        /// Numeric parameters in wire order.
        params: Vec<String>,
    },
}

impl ServerError {
    #[doc(hidden)]
    #[must_use]
    pub fn __standard(
        command: String,
        code: String,
        context: Vec<String>,
        description: String,
    ) -> Self {
        #[cfg(feature = "unstable")]
        let refusal = Refusal::from_standard_enabled(&command, &code, &BTreeSet::new());
        Self::Standard {
            command,
            code,
            context,
            description,
            #[cfg(feature = "unstable")]
            refusal,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __numeric(numeric: Numeric, params: Vec<String>) -> Self {
        Self::Numeric { numeric, params }
    }

    /// Classifies this refusal independently of its wire representation.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        match self {
            Self::Numeric { numeric, .. } => Refusal::from_numeric(*numeric),
            #[cfg(feature = "unstable")]
            Self::Standard { refusal, .. } => *refusal,
            #[cfg(not(feature = "unstable"))]
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
    /// Classification recorded using the negotiated unstable features.
    #[cfg(feature = "unstable")]
    #[doc(hidden)]
    pub refusal: Refusal,
}

impl StandardReply {
    /// Returns the refusal category when this is a failure.
    #[must_use]
    pub fn refusal(&self) -> Option<Refusal> {
        (self.kind == StandardReplyKind::Fail).then(|| {
            #[cfg(feature = "unstable")]
            return self.refusal;
            #[cfg(not(feature = "unstable"))]
            Refusal::from_standard(&self.command, &self.code)
        })
    }
}

/// One line in a history page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryLine {
    /// The replayed IRC line.
    pub line: Line,
    /// Whether the server supplied it as context outside the requested limit.
    pub context: bool,
    /// Whether the line's actor is this session's user.
    pub authorship: Authorship,
    /// Whether the line's distinct subject is this session's user.
    pub subject_authorship: Authorship,
}

/// Tags carried by a server line that identify and contextualize its event.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LineEnvelope {
    msgid: Option<String>,
    server_time: Option<std::time::SystemTime>,
    account: Option<String>,
}

impl LineEnvelope {
    pub(crate) fn from_line(line: &LineRef<'_>) -> Self {
        Self {
            msgid: line.tag("msgid").map(std::borrow::Cow::into_owned),
            server_time: line
                .tag("time")
                .and_then(|value| papo_proto::time::parse(&value)),
            account: line.tag("account").map(std::borrow::Cow::into_owned),
        }
    }

    /// Returns the server-assigned message identifier, when supplied.
    #[must_use]
    pub fn msgid(&self) -> Option<&str> {
        self.msgid.as_deref()
    }

    /// Returns the valid server timestamp, when supplied.
    #[must_use]
    pub const fn server_time(&self) -> Option<std::time::SystemTime> {
        self.server_time
    }

    /// Returns the actor's authenticated account, when tagged.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }
}

impl std::ops::Deref for HistoryLine {
    type Target = Line;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

/// One replayed entry in a history page.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HistoryItem {
    /// A `PRIVMSG` or `NOTICE`.
    Message(HistoryLine),
    /// A reaction or unreaction carried by `TAGMSG`.
    Reacted(HistoryLine),
    /// A `REDACT` command.
    Redacted(HistoryLine),
    /// A `TAGMSG` without reaction metadata.
    TagMsg(HistoryLine),
    /// A `JOIN` command.
    Joined(HistoryLine),
    /// A `PART` command.
    Parted(HistoryLine),
    /// A `QUIT` command.
    Quit(HistoryLine),
    /// A `KICK` command.
    Kicked(HistoryLine),
    /// A `NICK` command.
    NickChanged(HistoryLine),
    /// A `TOPIC` command.
    TopicChanged(HistoryLine),
    /// A `MODE` command.
    ModeChanged(HistoryLine),
    /// A replayed command without a more specific classification.
    ///
    /// A later minor release may refine a command currently classified here.
    Other(HistoryLine),
}

macro_rules! history_item_accessors {
    ($(($variant:ident, $method:ident)),+ $(,)?) => {
        $(
            #[doc = concat!("Returns the line when this item is [`HistoryItem::", stringify!($variant), "`].")]
            #[must_use]
            pub const fn $method(&self) -> Option<&HistoryLine> {
                if let Self::$variant(line) = self {
                    Some(line)
                } else {
                    None
                }
            }
        )+
    };
}

impl HistoryItem {
    fn line_mut(&mut self) -> &mut HistoryLine {
        match self {
            Self::Message(line)
            | Self::Reacted(line)
            | Self::Redacted(line)
            | Self::TagMsg(line)
            | Self::Joined(line)
            | Self::Parted(line)
            | Self::Quit(line)
            | Self::Kicked(line)
            | Self::NickChanged(line)
            | Self::TopicChanged(line)
            | Self::ModeChanged(line)
            | Self::Other(line) => line,
        }
    }

    history_item_accessors! {
        (Message, as_message),
        (Reacted, as_reacted),
        (Redacted, as_redacted),
        (TagMsg, as_tag_msg),
        (Joined, as_joined),
        (Parted, as_parted),
        (Quit, as_quit),
        (Kicked, as_kicked),
        (NickChanged, as_nick_changed),
        (TopicChanged, as_topic_changed),
        (ModeChanged, as_mode_changed),
        (Other, as_other),
    }

    /// Returns the chat message, or `None` for a replayed activity item.
    #[must_use]
    pub fn into_message(self) -> Option<HistoryLine> {
        match self {
            Self::Message(message) => Some(message),
            _ => None,
        }
    }

    /// Returns the replayed wire line and its history context flag.
    #[must_use]
    pub fn into_line(self) -> HistoryLine {
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
    /// Conversation named by the history batch.
    pub conversation: Conversation,
    /// Replayed items in server delivery order.
    pub items: Vec<HistoryItem>,
    /// Whether the server reported that no further page exists.
    ///
    /// A page shorter than the requested limit does not imply the end.
    pub end: bool,
}

impl HistoryPage {
    /// Iterates over chat messages, excluding replayed activity items.
    pub fn messages(&self) -> impl Iterator<Item = &HistoryLine> {
        self.items.iter().filter_map(HistoryItem::as_message)
    }
}

/// Compatibility name for a history page attached to a response.
pub type ResponseHistory = HistoryPage;

/// A completed `IRCv3` multiline message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultilineMessage {
    /// The reconstructed `PRIVMSG` or `NOTICE`.
    pub message: ReceivedLine,
}

/// A received `TAGMSG` and its client-only metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagMessage {
    /// Identity, time, and account tags from the line.
    pub envelope: LineEnvelope,
    /// Actor prefix without its leading colon.
    pub actor: String,
    /// Whether the actor is this session's user.
    pub authorship: Authorship,
    /// IRC conversation addressed by the message.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// Advertised `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
    /// Tags in wire order.
    pub tags: Vec<(String, Option<String>)>,
}

/// A server-relayed message redaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Redacted {
    /// Identity, time, and account tags from the line.
    pub envelope: LineEnvelope,
    /// Actor prefix without its leading colon.
    pub actor: String,
    /// Whether the actor is this session's user.
    pub authorship: Authorship,
    /// IRC conversation containing the message.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub conversation: Conversation,
    /// Advertised `STATUSMSG` prefixes on the wire address, in wire order.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    pub status: Option<Vec<char>>,
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
    /// Credentials require an explicit opt-in on this plaintext connection.
    InsecureAuthentication,
    /// An optional protocol feature is unavailable.
    Unsupported(Unsupported),
    /// The server cannot accept the supplied history reference.
    InvalidMessageReference,
    /// Account creation requires an email address.
    EmailRequired,
    /// The server requires the account name to match the current nickname.
    CustomAccountNameUnsupported,
    /// A metadata key is invalid.
    #[non_exhaustive]
    InvalidMetadataKey {
        /// The rejected key.
        key: String,
        /// Why the key was rejected.
        reason: papo_proto::InvalidIdentifier,
    },
    /// A metadata value exceeds the server's advertised byte limit.
    #[non_exhaustive]
    MetadataValueTooLong {
        /// Advertised maximum.
        maximum: usize,
        /// Supplied byte length.
        actual: usize,
    },
    /// A metadata subscription request exceeds the server's advertised limit.
    #[non_exhaustive]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CloseKind {
    /// Required SASL authentication failed.
    SaslFailed,
    /// Required SASL authentication was unavailable or timed out.
    SaslUnavailable,
    /// The server rejected the gateway's `WEBIRC` command.
    WebIrcRejected,
    /// Connection registration did not complete before its deadline.
    WelcomeTimeout,
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
    /// The server rejected this connection class (`463`).
    ConnectionRestricted,
    /// The server banned the connection (`465`).
    Banned,
    /// Every configured connection nickname was rejected.
    NickCandidatesExhausted,
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
            Self::InsecureAuthentication => formatter
                .write_str("credentials require an explicit opt-in on a plaintext connection"),
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
            | Self::InsecureAuthentication
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
    #[doc(hidden)]
    #[must_use]
    pub fn __invalid_metadata_key(key: String, reason: papo_proto::InvalidIdentifier) -> Self {
        Self::InvalidMetadataKey { key, reason }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __metadata_value_too_long(maximum: usize, actual: usize) -> Self {
        Self::MetadataValueTooLong { maximum, actual }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __too_many_metadata_subscriptions(maximum: usize, actual: usize) -> Self {
        Self::TooManyMetadataSubscriptions { maximum, actual }
    }

    fn unsupported(feature: Feature) -> Self {
        Self::Unsupported(Unsupported::new(feature, UnsupportedReason::NotOffered))
    }
}

/// A phase of establishing an IRC connection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ConnectPhase {
    /// Opening the underlying transport.
    Transport,
    /// Negotiating transport security.
    Tls,
    /// Discovering and enabling IRC capabilities.
    Capabilities,
    /// Authenticating the connection.
    Authenticating,
    /// Waiting for the server's welcome reply.
    WaitingForWelcome,
}

/// An event emitted by an IRC session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The IRC connection is ready for use.
    Connected(Welcome),
    /// A complete message of the day received during connection registration.
    #[non_exhaustive]
    Motd {
        /// Text from each `RPL_MOTD` numeric, in wire order.
        lines: Vec<String>,
    },
    /// A parsed protocol line not consumed by session protocol behavior.
    Line(ReceivedLine),
    /// A completed `draft/multiline` batch.
    MultilineMessage(MultilineMessage),
    /// A received tag-only message.
    TagMessage(TagMessage),
    /// A user's typing state changed.
    TypingChanged(TypingChanged),
    /// A message was redacted from a conversation's history.
    Redacted(Redacted),
    /// A request was sent without protocol-level response correlation.
    #[non_exhaustive]
    RequestUntracked {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request received its complete logical response.
    Response(Response),
    /// A labelled request did not receive a response before its deadline.
    #[non_exhaustive]
    RequestTimedOut {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request was pending when the connection ended.
    #[non_exhaustive]
    RequestAborted {
        /// Identifier assigned to the request.
        id: RequestId,
        /// Whether transmission of the request had begun.
        transmitted: bool,
    },
    /// A user-originated line was discarded before reaching the transport.
    #[doc(hidden)]
    #[non_exhaustive]
    TransmitAborted {
        /// Identifier assigned when the line was queued.
        token: TransmitToken,
    },
    /// A pending request's required feature became unavailable.
    #[non_exhaustive]
    RequestUnsupported {
        /// Identifier assigned to the request.
        id: RequestId,
        /// Feature and current reason for its unavailability.
        unsupported: Unsupported,
    },
    /// An unsolicited `IRCv3` standard reply.
    StandardReply(StandardReply),
    /// Account creation completed and the session is authenticated.
    #[non_exhaustive]
    AccountCreated {
        /// The created account name.
        account: AccountName,
        /// Whether the server authenticated the session to the account.
        logged_in: bool,
    },
    /// Account creation requires a verification step.
    #[non_exhaustive]
    VerificationRequired {
        /// The account awaiting verification.
        account: AccountName,
        /// Human-readable instructions from the server.
        message: String,
    },
    /// The server reported the account's persistence state.
    #[cfg(feature = "unstable")]
    PersistenceStatus(PersistenceStatus),
    /// An upstream bouncer network was added, updated, or removed.
    #[cfg(feature = "unstable")]
    #[non_exhaustive]
    BouncerNetworkChanged {
        /// Bouncer-assigned network identifier.
        id: NetworkId,
        /// Complete cached network state, or `None` when removed.
        network: Option<BouncerNetwork>,
    },
    /// A user or channel's cached metadata changed.
    #[non_exhaustive]
    MetadataChanged {
        /// User or channel whose metadata changed.
        subject: Subject,
        /// Metadata key.
        key: String,
        /// New server-returned value, or `None` when cleared.
        value: Option<String>,
        /// Visibility policy reported by the server.
        visibility: Visibility,
    },
    /// The server deferred metadata synchronization for a user or channel.
    #[non_exhaustive]
    MetadataSyncLater {
        /// Nickname or channel whose synchronization was deferred.
        target: String,
        /// Suggested delay before retrying, when supplied.
        retry_after: Option<Duration>,
    },
    /// A completed history or playback batch.
    #[non_exhaustive]
    History {
        /// The conversation named by the batch.
        conversation: Conversation,
        /// Replayed items in server delivery order.
        items: Vec<HistoryItem>,
        /// Whether the server reported that no further page exists.
        end: bool,
    },
    /// Targets that received messages during a requested time range.
    HistoryTargets(Vec<(Conversation, std::time::SystemTime)>),
    /// A conversation's read marker advanced or was first reported.
    ReadMarkerChanged(ReadMarker),
    /// Users departed because connectivity between two servers was lost.
    ///
    /// This event is emitted for an empty batch. Non-`QUIT` lines are delivered as
    /// separate [`Event::Line`] values.
    #[non_exhaustive]
    Netsplit {
        /// The servers on either side of the lost link.
        servers: [String; 2],
        /// The `QUIT` messages grouped by the server.
        quits: Vec<Line>,
        /// Authorship facts corresponding to `quits` in wire order.
        authorship: Vec<Authorship>,
        /// Shared channels captured before each user was removed, in wire order.
        channels: Vec<Vec<ChannelName>>,
    },
    /// Users returned after connectivity between two servers was restored.
    ///
    /// This event is emitted for an empty batch. Non-`JOIN` lines are delivered as
    /// separate [`Event::Line`] values.
    #[non_exhaustive]
    Netjoin {
        /// The servers on either side of the restored link.
        servers: [String; 2],
        /// The `JOIN` messages grouped by the server.
        joins: Vec<Line>,
        /// Authorship facts corresponding to `joins` in wire order.
        authorship: Vec<Authorship>,
    },
    /// A server batch exceeded a retention bound or deadline.
    #[non_exhaustive]
    BatchAbandoned {
        /// The opaque batch reference.
        reference: String,
        /// The batch type from its opening line.
        batch_type: String,
        /// The bound that was reached.
        reason: BatchAbandonReason,
    },
    /// A line that could not be decoded or parsed.
    #[non_exhaustive]
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// A request for the transport to drop the connection.
    #[non_exhaustive]
    Close {
        /// Machine-readable closure classification.
        kind: CloseKind,
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// Configured authentication failed and connecting is continuing.
    #[non_exhaustive]
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// The capabilities enabled on an established connection changed.
    #[non_exhaustive]
    CapsChanged {
        /// Newly enabled capabilities and updated advertised values.
        added: Capabilities,
        /// Capabilities removed by the server.
        removed: Capabilities,
    },
    /// Optional behaviors available on an established connection changed.
    #[non_exhaustive]
    FeaturesChanged {
        /// Behaviors that became available.
        added: Vec<Feature>,
        /// Behaviors that became unavailable.
        removed: Vec<Feature>,
    },
    /// A connection advertised name rules different from the remembered rules.
    #[non_exhaustive]
    CasemappingChanged {
        /// Rules retained from the previous connection or offline seed.
        old: Casemapping,
        /// Rules advertised by the current connection.
        new: Casemapping,
    },
    /// A plaintext connection must be replaced with TLS on `port`.
    #[non_exhaustive]
    StsUpgrade {
        /// The advertised TLS port.
        port: u16,
    },
    /// An STS persistence action for the transport shell to store.
    StsPolicy(sts::StsAction),
    /// Server input exceeded a configured state-retention bound.
    #[non_exhaustive]
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// A monitored nickname came online.
    #[non_exhaustive]
    MonitorOnline {
        /// The monitored nickname.
        nick: Nick,
        /// The username from `nick!user@host`, when supplied by the server; it may start with `~`.
        username: Option<String>,
        /// The user's current hostname, when supplied by the server.
        host: Option<String>,
    },
    /// A monitored nickname went offline.
    #[non_exhaustive]
    MonitorOffline {
        /// The monitored nickname.
        nick: Nick,
    },
    /// The server rejected nicknames because its monitor list was full.
    #[non_exhaustive]
    MonitorListFull {
        /// The server's advertised monitor-list limit.
        limit: usize,
        /// Nicknames the server did not add.
        nicks: Vec<Nick>,
    },
    /// An automatic action could not use an optional server feature.
    Unsupported(Unsupported),
    /// A user joined a channel shared with the session.
    #[non_exhaustive]
    Joined {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// The joining user's prefix.
        actor: String,
        /// The joining user's known account state.
        account: Account,
        /// The joining user's real name, when supplied by the server.
        realname: Option<String>,
        /// The channel that was joined.
        channel: ChannelName,
        /// The joining user's nickname.
        nick: Nick,
        /// Whether the joining user is this session's user.
        authorship: Authorship,
    },
    /// Initial channel membership synchronization completed.
    #[non_exhaustive]
    ChannelSynced {
        /// The synchronized channel.
        channel: ChannelName,
    },
    /// A channel's member facts were refreshed by a `WHO` sweep.
    #[non_exhaustive]
    MembersRefreshed {
        /// The refreshed channel.
        channel: ChannelName,
    },
    /// A user left a shared channel with `PART`.
    #[non_exhaustive]
    Parted {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// The departing user's prefix.
        actor: String,
        /// The departed channel.
        channel: ChannelName,
        /// The departing user's nickname.
        nick: Nick,
        /// The optional part reason.
        reason: Option<String>,
        /// Whether the departing user is this session's user.
        authorship: Authorship,
    },
    /// A user was removed from a shared channel with `KICK`.
    #[non_exhaustive]
    Kicked {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// The affected channel.
        channel: ChannelName,
        /// The removed user.
        subject: Nick,
        /// The user or server that issued the kick.
        actor: String,
        /// Whether the actor is this session's user.
        authorship: Authorship,
        /// Whether the removed user is this session's user.
        subject_authorship: Authorship,
        /// The optional kick reason.
        reason: Option<String>,
    },
    /// A user was invited to a channel.
    #[non_exhaustive]
    Invited {
        /// The invited channel.
        channel: ChannelName,
        /// The invited user.
        subject: Nick,
        /// The user or server prefix that sent the invitation.
        actor: String,
        /// Whether the actor is this session's user.
        authorship: Authorship,
        /// Whether the invited user is this session's user.
        subject_authorship: Authorship,
    },
    /// A user sharing channels with the session disconnected.
    #[non_exhaustive]
    Quit {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// The disconnected user's prefix.
        actor: String,
        /// The disconnected user's nickname.
        nick: Nick,
        /// The optional quit reason.
        reason: Option<String>,
        /// Shared channels captured before the user was removed.
        channels: Vec<ChannelName>,
        /// Whether the disconnected user is this session's user.
        authorship: Authorship,
    },
    /// A user changed nickname.
    #[non_exhaustive]
    NickChanged {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// The renamed user's prefix before the change.
        actor: String,
        /// The previous nickname.
        old: Nick,
        /// The new nickname.
        new: Nick,
        /// Whether the renamed user is this session's user.
        authorship: Authorship,
    },
    /// A joined channel was renamed.
    #[non_exhaustive]
    ChannelRenamed {
        /// The previous channel name.
        old: ChannelName,
        /// The new channel name.
        new: ChannelName,
        /// The user or server prefix that renamed the channel.
        actor: String,
        /// Whether the actor is this session's user.
        authorship: Authorship,
        /// The server-supplied reason, when present and non-empty.
        reason: Option<String>,
    },
    /// A user's account state changed.
    #[non_exhaustive]
    AccountChanged {
        /// The affected user's prefix.
        actor: String,
        /// The affected user's nickname.
        nick: Nick,
        /// The user's new account state.
        account: Account,
    },
    /// A user's away state changed.
    #[non_exhaustive]
    AwayChanged {
        /// The affected user's prefix.
        actor: String,
        /// The affected user's nickname.
        nick: Nick,
        /// The user's new presence state.
        away: Away,
    },
    /// A user's username or hostname changed.
    #[non_exhaustive]
    HostChanged {
        /// The affected user's prefix when known, otherwise their nickname.
        actor: String,
        /// The affected user's nickname.
        nick: Nick,
        /// The new username from `nick!user@host`; it may start with `~`.
        username: String,
        /// The user's new hostname.
        host: String,
    },
    /// A user's real name changed.
    #[non_exhaustive]
    RealnameChanged {
        /// The affected user's prefix.
        actor: String,
        /// The affected user's nickname.
        nick: Nick,
        /// The user's new real name.
        realname: String,
    },
    /// A user or channel mode changed.
    #[non_exhaustive]
    ModeChanged {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// User or channel whose modes changed.
        subject: Subject,
        /// The user or server that changed the modes.
        actor: String,
        /// Whether the actor is this session's user.
        authorship: Authorship,
        /// Whether the distinct subject is this session's user.
        subject_authorship: Authorship,
        /// Parsed mode operations in wire order.
        changes: Vec<ModeChange>,
    },
    /// A channel topic changed live.
    #[non_exhaustive]
    TopicChanged {
        /// Identity, time, and account tags from the line.
        envelope: LineEnvelope,
        /// The affected channel.
        channel: ChannelName,
        /// The new topic, or `None` when it was cleared.
        topic: Option<String>,
        /// The user or server that changed the topic.
        actor: String,
        /// Whether the actor is this session's user.
        authorship: Authorship,
    },
}

macro_rules! hidden_event_constructors {
    ($($name:ident($($field:ident: $ty:ty),*) => $variant:ident;)*) => {
        impl Event {
            $(
                #[doc(hidden)]
                #[must_use]
                pub fn $name($($field: $ty),*) -> Self {
                    Self::$variant { $($field),* }
                }
            )*
        }
    };
}

hidden_event_constructors! {
    __malformed(line: Vec<u8>) => Malformed;
    __features_changed(added: Vec<Feature>, removed: Vec<Feature>) => FeaturesChanged;
    __casemapping_changed(old: Casemapping, new: Casemapping) => CasemappingChanged;
    __invited(channel: ChannelName, subject: Nick, actor: String, authorship: Authorship, subject_authorship: Authorship) => Invited;
    __joined(envelope: LineEnvelope, actor: String, account: Account, realname: Option<String>, channel: ChannelName, nick: Nick, authorship: Authorship) => Joined;
    __account_created(account: AccountName, logged_in: bool) => AccountCreated;
    __verification_required(account: AccountName, message: String) => VerificationRequired;
    __channel_renamed(old: ChannelName, new: ChannelName, actor: String, authorship: Authorship, reason: Option<String>) => ChannelRenamed;
    __metadata_changed(subject: Subject, key: String, value: Option<String>, visibility: Visibility) => MetadataChanged;
    __metadata_sync_later(target: String, retry_after: Option<Duration>) => MetadataSyncLater;
    __channel_synced(channel: ChannelName) => ChannelSynced;
    __members_refreshed(channel: ChannelName) => MembersRefreshed;
    __parted(envelope: LineEnvelope, channel: ChannelName, actor: String, nick: Nick, reason: Option<String>, authorship: Authorship) => Parted;
    __kicked(envelope: LineEnvelope, channel: ChannelName, subject: Nick, actor: String, authorship: Authorship, subject_authorship: Authorship, reason: Option<String>) => Kicked;
    __quit(envelope: LineEnvelope, actor: String, nick: Nick, reason: Option<String>, channels: Vec<ChannelName>, authorship: Authorship) => Quit;
    __nick_changed(envelope: LineEnvelope, actor: String, old: Nick, new: Nick, authorship: Authorship) => NickChanged;
    __account_changed(actor: String, nick: Nick, account: Account) => AccountChanged;
    __away_changed(actor: String, nick: Nick, away: Away) => AwayChanged;
    __host_changed(actor: String, nick: Nick, username: String, host: String) => HostChanged;
    __realname_changed(actor: String, nick: Nick, realname: String) => RealnameChanged;
    __mode_changed(envelope: LineEnvelope, subject: Subject, actor: String, authorship: Authorship, subject_authorship: Authorship, changes: Vec<ModeChange>) => ModeChanged;
    __topic_changed(envelope: LineEnvelope, channel: ChannelName, topic: Option<String>, actor: String, authorship: Authorship) => TopicChanged;
    __state_limit_reached(limit: StateLimit) => StateLimitReached;
    __monitor_online(nick: Nick, username: Option<String>, host: Option<String>) => MonitorOnline;
    __monitor_offline(nick: Nick) => MonitorOffline;
    __monitor_list_full(limit: usize, nicks: Vec<Nick>) => MonitorListFull;
    __history(conversation: Conversation, items: Vec<HistoryItem>, end: bool) => History;
    __netsplit(servers: [String; 2], quits: Vec<Line>, authorship: Vec<Authorship>, channels: Vec<Vec<ChannelName>>) => Netsplit;
    __netjoin(servers: [String; 2], joins: Vec<Line>, authorship: Vec<Authorship>) => Netjoin;
    __batch_abandoned(reference: String, batch_type: String, reason: BatchAbandonReason) => BatchAbandoned;
}

#[cfg(feature = "unstable")]
impl Event {
    #[doc(hidden)]
    #[must_use]
    pub fn __bouncer_network_changed(id: NetworkId, network: Option<BouncerNetwork>) -> Self {
        Self::BouncerNetworkChanged { id, network }
    }
}

/// Why an incomplete server batch was released as individual messages.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

/// A parsed line annotated with protocol dispatch context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedLine {
    line: Line,
    batch_type: Option<String>,
    authorship: Authorship,
    conversation: Option<Conversation>,
    status: Option<Vec<char>>,
}

impl ReceivedLine {
    /// Borrows the parsed IRC line.
    #[must_use]
    pub fn as_ref(&self) -> LineRef<'_> {
        self.line.as_ref()
    }

    /// Returns the containing batch's type, when known.
    #[must_use]
    pub fn batch_type(&self) -> Option<&str> {
        self.batch_type.as_deref()
    }

    /// Returns whether the line's actor is this session's user.
    #[must_use]
    pub const fn authorship(&self) -> Authorship {
        self.authorship
    }

    /// Returns the conversation addressed by this line, when its address is valid.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub const fn conversation(&self) -> Option<&Conversation> {
        self.conversation.as_ref()
    }

    /// Returns the advertised `STATUSMSG` prefixes removed from the line's address.
    ///
    /// See [`Conversation`] for status-prefixed line guidance.
    #[must_use]
    pub fn status(&self) -> Option<&[char]> {
        self.status.as_deref()
    }

    /// Removes the dispatch annotation and returns the parsed IRC line.
    #[must_use]
    pub fn into_line(self) -> Line {
        self.line
    }

    /// Splits the line into its wire value and dispatch annotations.
    #[must_use]
    pub fn into_parts(self) -> (Line, Option<String>) {
        (self.line, self.batch_type)
    }
}

impl std::ops::Deref for ReceivedLine {
    type Target = Line;

    fn deref(&self) -> &Self::Target {
        &self.line
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
    /// Entries retained for one channel list mode.
    ListEntries,
    /// Targets retained in the metadata cache.
    MetadataTargets,
    /// Metadata keys retained for one user or channel.
    MetadataKeys,
    /// Upstream bouncer networks.
    #[cfg(feature = "unstable")]
    BouncerNetworks,
    /// ISUPPORT tokens advertised by the server.
    IsupportTokens,
}

type PendingRequests = BTreeMap<String, (RequestId, Option<Instant>, bool, Option<Feature>, bool)>;

/// Sans-I/O state for one IRC client session.
pub struct Session {
    account_registration: AccountRegistration,
    away_on_connect_sent: bool,
    away_messages: BTreeMap<RequestId, String>,
    batches: Batches,
    config: Config,
    connection: Connection,
    caps: Caps,
    casemapping_baseline: Casemapping,
    casemapping_change_emitted: bool,
    ctcp_rate_limit: RateLimiter,
    ctcp_sources: VecDeque<(String, Instant)>,
    framer: Framer,
    isupport: IsupportState,
    keepalive: Keepalive,
    monitor: Monitor,
    nick_regain: NickRegain,
    metadata_subscriptions_sent: bool,
    rate_limit: RateLimiter,
    remembered_casemapping: Casemapping,
    requests: PendingRequests,
    unlabelled_requests: VecDeque<UnlabelledRequest>,
    next_request_id: u64,
    next_transmit_token: u64,
    welcome_state: WelcomeState,
    sasl: SaslSession,
    state: State,
    state_generation: u64,
    typing_generation: u64,
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
            .field("casemapping_baseline", &self.casemapping_baseline)
            .field(
                "casemapping_change_emitted",
                &self.casemapping_change_emitted,
            )
            .field("ctcp_rate_limit", &self.ctcp_rate_limit)
            .field("ctcp_sources", &self.ctcp_sources)
            .field("framer", &self.framer)
            .field("isupport", &self.isupport)
            .field("keepalive", &self.keepalive)
            .field("monitor", &self.monitor)
            .field("nick_regain", &self.nick_regain)
            .field(
                "metadata_subscriptions_sent",
                &self.metadata_subscriptions_sent,
            )
            .field("rate_limit", &self.rate_limit)
            .field("remembered_casemapping", &self.remembered_casemapping)
            .field("requests", &self.requests)
            .field("unlabelled_requests", &self.unlabelled_requests)
            .field("next_request_id", &self.next_request_id)
            .field("next_transmit_token", &self.next_transmit_token)
            .field("welcome_state", &self.welcome_state)
            .field(cap::SASL.as_str(), &self.sasl)
            .field("state", &self.state)
            .field("state_generation", &self.state_generation)
            .field("typing_generation", &self.typing_generation)
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
    immediate: VecDeque<QueuedLine>,
    events: VecDeque<Event>,
    connect_phases: VecDeque<ConnectPhase>,
    close_requested: bool,
}

#[derive(Debug)]
enum Transmission {
    Line(QueuedLine),
    Batch {
        lines: VecDeque<WireLine>,
        started: bool,
        token: Option<TransmitToken>,
        request: Option<RequestId>,
    },
}

#[derive(Debug)]
struct QueuedLine {
    line: WireLine,
    token: Option<TransmitToken>,
    request: Option<RequestId>,
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
    fn transmit_raw(&mut self, line: WireLine) {
        self.transmit_tagged(line, None);
    }

    fn transmit_tagged(&mut self, line: WireLine, token: Option<TransmitToken>) {
        self.transmit_tracked(line, token, None);
    }

    fn transmit_tracked(
        &mut self,
        line: WireLine,
        token: Option<TransmitToken>,
        request: Option<RequestId>,
    ) {
        if self.close_requested {
            if let Some(token) = token {
                self.event(Event::TransmitAborted { token });
            }
            return;
        }
        let line = QueuedLine {
            line,
            token,
            request,
        };
        if bypasses_rate_limit(&line.line) {
            self.immediate.push_back(line);
        } else {
            self.transmits.push_back(Transmission::Line(line));
        }
    }

    fn transmit_ordered(&mut self, line: WireLine, request: Option<RequestId>) {
        if !self.close_requested {
            self.transmits.push_back(Transmission::Line(QueuedLine {
                line,
                token: None,
                request,
            }));
        }
    }

    fn transmit_tracked_batch(
        &mut self,
        lines: Vec<WireLine>,
        token: Option<TransmitToken>,
        request: Option<RequestId>,
    ) {
        if !self.close_requested && !lines.is_empty() {
            self.transmits.push_back(Transmission::Batch {
                lines: lines.into(),
                started: false,
                token,
                request,
            });
        } else if let Some(token) = token {
            self.event(Event::TransmitAborted { token });
        }
    }

    fn transmit<const N: usize>(&mut self, command: &str, params: [&str; N]) {
        match Line::new(command).with_params(params).encode() {
            Ok(line) => self.transmit_raw(line),
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
        match Line::new(command).with_params(params).encode() {
            Ok(line) => self.immediate.push_back(QueuedLine {
                line,
                token: None,
                request: None,
            }),
            Err(error) => self.close(
                CloseKind::Other,
                format!("invalid outbound message: {error}"),
            ),
        }
    }

    fn event(&mut self, event: Event) {
        self.events.push_back(event);
    }

    fn connect_phase(&mut self, phase: ConnectPhase) {
        self.connect_phases.push_back(phase);
    }

    fn close(&mut self, kind: CloseKind, reason: String) {
        self.close_requested = true;
        self.event(Event::Close { kind, reason });
    }

    fn close_and_discard(&mut self, kind: CloseKind, reason: String) {
        self.abort_transmits();
        self.close(kind, reason);
    }

    fn reset_connection(&mut self) {
        self.abort_transmits();
        self.close_requested = false;
    }

    fn abort_transmits(&mut self) {
        let tokens = self
            .immediate
            .drain(..)
            .filter_map(|line| line.token)
            .chain(
                self.transmits
                    .drain(..)
                    .filter_map(|transmit| match transmit {
                        Transmission::Line(line) => line.token,
                        Transmission::Batch { token, .. } => token,
                    }),
            )
            .collect::<Vec<_>>();
        for token in tokens {
            self.event(Event::TransmitAborted { token });
        }
    }
}

impl Session {
    fn authorship(&self, message: &LineRef<'_>) -> Authorship {
        let same_account = message.tag("account").is_some_and(|actor| {
            self.account().is_some_and(|own| {
                let mapping = self.isupport.value().casemapping();
                mapping.account(actor.as_ref()) == mapping.account(own)
            })
        });
        if self.is_own_source(message) || same_account {
            Authorship::Own
        } else {
            Authorship::Other
        }
    }

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
        let remembered_casemapping = config.casemapping;
        let connection = Connection {
            host: config.host.clone(),
            security: config.connection_security,
        };
        let welcome_state = WelcomeState::new(&config);
        let isupport = IsupportState::new(config.decoding);
        let sasl = SaslSession::new(&config, entropy);
        let rate_limit = RateLimiter::new(config.rate_limit);
        let ctcp_rate_limit = RateLimiter::new(config.ctcp.rate);
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
            casemapping_baseline: remembered_casemapping,
            casemapping_change_emitted: false,
            ctcp_rate_limit,
            ctcp_sources: VecDeque::new(),
            framer: Framer::default(),
            isupport,
            keepalive: Keepalive::default(),
            monitor: Monitor::default(),
            nick_regain: NickRegain::default(),
            metadata_subscriptions_sent: false,
            rate_limit,
            remembered_casemapping,
            requests: BTreeMap::new(),
            unlabelled_requests: VecDeque::new(),
            next_request_id: 0,
            next_transmit_token: 0,
            welcome_state,
            sasl,
            state,
            state_generation: 0,
            typing_generation: 0,
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
        if self.connection.security == ConnectionSecurity::Plaintext
            && !self.config.allow_insecure_auth
            && (self.config.password.is_some()
                || self.config.webirc.is_some()
                || self.config.sasl.is_some()
                || self.config.registration_on_connect.is_some())
        {
            self.outbox.close(
                CloseKind::Other,
                "credentials require an explicit opt-in on a plaintext connection".to_owned(),
            );
            return;
        }
        self.batches.reset();
        self.ctcp_rate_limit.reset(now);
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
        self.casemapping_baseline = self.remembered_casemapping;
        self.casemapping_change_emitted = false;
        self.isupport.handle_connected();
        self.sasl.handle_connected();
        self.nick_regain.reset();
        self.state
            .reset(&self.config, Casemapping::default(), self.isupport.value());
        self.state_generation = self.state_generation.wrapping_add(1);
        self.typing_generation = self.typing_generation.wrapping_add(1);
        self.keepalive
            .handle_connected(now, self.config.ping_interval, self.config.ping_timeout);
        self.outbox.connect_phase(ConnectPhase::Capabilities);
        self.welcome_state
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
        match LineRef::parse(&decoded) {
            Ok(message) => {
                let multiline_max_bytes = outgoing::multiline_limits(self.caps.enabled())
                    .map_or(usize::MAX, |limits| limits.max_bytes);
                match self.batches.handle(
                    now,
                    &message,
                    multiline_max_bytes,
                    self.caps.bouncer_enabled(),
                    message
                        .tag("label")
                        .is_some_and(|label| self.requests.contains_key(label.as_ref())),
                ) {
                    BatchAction::Consumed => {}
                    BatchAction::Complete(events) => {
                        for event in events {
                            self.dispatch_batch_event(now, event);
                        }
                    }
                    BatchAction::Dispatch(context) => {
                        self.dispatch_message(now, &message, &context, true, None);
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
        self.handle_typing_timeout(now);
        let expired: BTreeSet<_> = self
            .requests
            .iter()
            .filter(|(_, (_, started, _, _, _))| {
                started
                    .and_then(|started| started.checked_add(self.config.request_timeout))
                    .is_some_and(|deadline| now >= deadline)
            })
            .map(|(_, (id, _, _, _, _))| *id)
            .collect();
        for id in expired {
            self.invalidate_list_request(id);
            self.requests
                .retain(|_, (pending, _, _, _, _)| *pending != id);
            self.unlabelled_requests.retain(|request| request.id != id);
            self.away_messages.remove(&id);
            self.outbox.event(Event::RequestTimedOut { id });
        }
        let pending = std::mem::take(&mut self.unlabelled_requests);
        let mut unlabelled_expired = false;
        for request in pending {
            if request
                .started
                .and_then(|started| started.checked_add(self.config.request_timeout))
                .is_some_and(|deadline| now >= deadline)
            {
                if let ResponseMatcher::ListMode { channel, mode } = &request.matcher {
                    self.state.invalidate_mode_list(channel, *mode);
                }
                self.away_messages.remove(&request.id);
                self.outbox.event(Event::RequestTimedOut { id: request.id });
                unlabelled_expired = true;
            } else {
                self.unlabelled_requests.push_back(request);
            }
        }
        if unlabelled_expired {
            self.start_deferred_who();
        }
        let was_connected = self.welcome_state.is_connected();
        self.welcome_state.handle_timeout(
            now,
            self.caps.enabled(),
            self.isupport.value(),
            &mut self.outbox,
        );
        if !was_connected && self.welcome_state.is_connected() {
            self.monitor
                .restore(self.isupport.value(), &mut self.outbox);
            self.nick_regain.start(
                now,
                &self.config,
                self.isupport.value(),
                &self.monitor,
                &mut self.outbox,
            );
        }
        self.sasl.handle_timeout(now, &mut self.outbox);
        self.maybe_subscribe_metadata(now);
        self.caps.maybe_finish(
            self.sasl.is_finished() && self.account_registration.authentication_finished(),
            self.config.selected_bouncer_network(),
            &mut self.outbox,
        );
        self.keepalive.handle_timeout(now, &mut self.outbox);
        self.nick_regain
            .handle_timeout(now, &self.config.nick, &mut self.outbox);
        self.apply_close_request();
    }

    fn invalidate_list_request(&mut self, id: RequestId) {
        let list = self
            .unlabelled_requests
            .iter()
            .find(|request| request.id == id)
            .and_then(|request| match &request.matcher {
                ResponseMatcher::ListMode { channel, mode } => Some((channel.clone(), *mode)),
                _ => None,
            });
        if let Some((channel, mode)) = list {
            self.state.invalidate_mode_list(&channel, mode);
        }
    }

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.framer.close();
        self.outbox.reset_connection();
        self.batches.reset();
        let mut labelled = BTreeMap::new();
        for (_, (id, started, _, _, _)) in std::mem::take(&mut self.requests) {
            self.unlabelled_requests.retain(|request| request.id != id);
            labelled
                .entry(id)
                .and_modify(|transmitted| *transmitted |= started.is_some())
                .or_insert_with(|| started.is_some());
        }
        for (id, transmitted) in labelled {
            self.outbox.event(Event::RequestAborted { id, transmitted });
        }
        for request in self.unlabelled_requests.drain(..) {
            self.outbox.event(Event::RequestAborted {
                id: request.id,
                transmitted: request.started.is_some(),
            });
        }
        self.away_messages.clear();
        self.welcome_state.handle_disconnected();
        self.account_registration.handle_disconnected();
        self.sasl.handle_disconnected();
        self.keepalive.handle_disconnected();
        self.monitor.handle_disconnected();
        self.nick_regain.reset();
        self.metadata_subscriptions_sent = false;
        let casemapping = self.state.casemapping();
        self.remembered_casemapping = casemapping;
        self.state
            .reset(&self.config, casemapping, self.isupport.value());
        self.sync_monitored_state();
        self.state_generation = self.state_generation.wrapping_add(1);
        self.typing_generation = self.typing_generation.wrapping_add(1);
    }

    /// Validates and queues a user-originated raw message for transmission.
    ///
    /// The caller paces transmission by draining the intentionally unbounded
    /// outbound queue through [`Session::poll_transmit`].
    /// Parameters appended from [`Secret`] are redacted and wiped. Parameters in a parsed raw
    /// [`Line`] are ordinary text, so a caller that parses a credential into a line owns that
    /// diagnostic and memory-handling choice.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if `message` cannot be represented as a valid IRC wire line.
    pub fn send(&mut self, now: Instant, message: Line) -> Result<(), BuildError> {
        self.send_tracked(now, message).map(drop)
    }

    /// Queues a user-originated line with a write-completion identifier.
    #[doc(hidden)]
    pub fn send_tracked(
        &mut self,
        now: Instant,
        message: Line,
    ) -> Result<TransmitToken, BuildError> {
        self.rate_limit.advance(now);
        let line = message.encode()?;
        drop(message);
        let token = TransmitToken(self.next_transmit_token);
        self.next_transmit_token = self.next_transmit_token.wrapping_add(1);
        self.outbox.transmit_tagged(line, Some(token));
        Ok(token)
    }

    fn text_budget(&self, kind: TextKind, target: &str) -> usize {
        outgoing::text_budget(&self.state, kind, target)
    }

    pub(crate) fn validate_target(&self, target: &str) -> Result<(), RequestError> {
        self.state
            .parse_target(target)
            .map(drop)
            .map_err(|_| RequestError::Build(BuildError::InvalidParam(0)))
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
        self.send_composed_text(now, kind, target, text, tags, false)
    }

    /// Queues CTCP ACTION text and reports whether line splitting was applied.
    #[doc(hidden)]
    pub fn send_action_detailed(
        &mut self,
        now: Instant,
        target: &str,
        text: &str,
        tags: &[(&str, &str)],
    ) -> Result<SendTextResult, RequestError> {
        self.send_composed_text(now, TextKind::Privmsg, target, text, tags, true)
    }

    fn send_composed_text(
        &mut self,
        now: Instant,
        kind: TextKind,
        target: &str,
        text: &str,
        tags: &[(&str, &str)],
        action: bool,
    ) -> Result<SendTextResult, RequestError> {
        self.validate_target(target)?;
        self.rate_limit.advance(now);
        let multiline = self
            .caps
            .enabled()
            .contains_key(&cap::BATCH)
            .then(|| outgoing::multiline_limits(self.caps.enabled()))
            .flatten();
        let budget = multiline.map_or(self.text_budget(kind, target), |limits| {
            self.text_budget(kind, target).min(limits.max_bytes)
        });
        let composed = outgoing::compose_text(text, budget, multiline.is_some(), action)?;
        let text = composed.as_deref().unwrap_or(text);
        let segments = papo_proto::split_text(text, budget);
        if segments.is_empty() {
            return Err(RequestError::EmptyText);
        }
        let labelled = self.caps.enabled().contains_key(&cap::LABELED_RESPONSE)
            && self.caps.enabled().contains_key(&cap::ECHO_MESSAGE);
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        let id = RequestId(self.next_request_id);
        let label_prefix = labelled.then(|| encode_label(self.next_request_id));
        let mut labels = Vec::new();
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
                let params = [format!("+{id}"), cap::MULTILINE.into(), target.into()];
                let mut open = Line::new("BATCH").with_params(params);
                for &(key, value) in tags {
                    if batch_count == 0 || !outgoing::first_line_only(key) {
                        open = open.with_tag(key, value);
                    }
                }
                if let Some(label) = text_label(label_prefix.as_deref(), batch_count) {
                    open = open.with_tag("label", &label);
                    labels.push(label);
                }
                lines.push(open.encode()?);
                for (offset, segment) in segments[start..end].iter().enumerate() {
                    let mut inner = Line::new(kind.command())
                        .with_params([target, segment.text])
                        .with_tag(cap::BATCH.as_str(), &id);
                    if offset > 0 && segments[offset + start - 1].concat {
                        inner = inner.with_tag("draft/multiline-concat", "");
                    }
                    lines.push(inner.encode()?);
                }
                lines.push(Line::new("BATCH").param(format!("-{id}")).encode()?);
                start = end;
                batch_count += 1;
            }
        } else {
            for (index, segment) in segments.iter().enumerate() {
                let mut message = Line::new(kind.command()).with_params([target, segment.text]);
                for &(key, value) in tags {
                    if index == 0 || !outgoing::first_line_only(key) {
                        message = message.with_tag(key, value);
                    }
                }
                if let Some(label) = text_label(label_prefix.as_deref(), index) {
                    message = message.with_tag("label", &label);
                    labels.push(label);
                }
                lines.push(message.encode()?);
            }
        }
        if self.requests.len().saturating_add(labels.len()) > MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        self.next_request_id = self.next_request_id.wrapping_add(1);
        for label in labels.iter().cloned() {
            self.requests.insert(label, (id, None, true, None, false));
        }
        let transmit =
            self.queue_text_lines(lines, batch_count > 0, !labelled, labelled.then_some(id));
        Ok(SendTextResult {
            request: labelled.then_some(id),
            transmit,
            confirmations: labels.len(),
            split: segments.len() > 1,
        })
    }

    fn queue_text_lines(
        &mut self,
        lines: Vec<WireLine>,
        batched: bool,
        tracked: bool,
        request: Option<RequestId>,
    ) -> Option<TransmitToken> {
        let token = tracked.then(|| {
            let token = TransmitToken(self.next_transmit_token);
            self.next_transmit_token = self.next_transmit_token.wrapping_add(1);
            token
        });
        if batched {
            self.outbox.transmit_tracked_batch(lines, token, request);
        } else {
            let last = lines.len() - 1;
            for (index, line) in lines.into_iter().enumerate() {
                self.outbox.transmit_tracked(
                    line,
                    (index == last).then_some(token).flatten(),
                    request,
                );
            }
        }
        token
    }

    /// Queues a message and assigns an identifier for its eventual response.
    ///
    /// When `labeled-response` is enabled, the message receives a fresh `label` tag and remains
    /// pending. Otherwise it is sent unchanged and [`Event::RequestUntracked`] is emitted.
    /// Parameters appended from [`Secret`] are redacted and wiped; parsed raw-line parameters are
    /// ordinary text and should not contain credentials.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when `labeled-response` or a feature explicitly
    /// requested by the message is unavailable,
    /// [`RequestError::PendingLimit`] when 256 labelled requests are already pending, or
    /// [`RequestError::Build`] when the tagged message cannot be represented on the wire.
    pub fn request(&mut self, now: Instant, message: Line) -> Result<RequestId, RequestError> {
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
        message: Line,
    ) -> Result<RequestId, RequestError> {
        self.request_with_event_policy(now, message, false)
    }

    fn request_with_event_policy(
        &mut self,
        now: Instant,
        message: Line,
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
        message: Line,
        feature: Feature,
    ) -> Result<RequestId, RequestError> {
        self.require_feature(feature)?;
        self.queue_request(now, message, Some(feature), true)
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
        message: Line,
        matcher: ResponseMatcher,
    ) -> Result<RequestId, RequestError> {
        let who_sweep = match &matcher {
            ResponseMatcher::Who(channel)
                if message.as_ref().command() == Command::Who
                    && self.state.channel(channel).is_some() =>
            {
                Some(channel.clone())
            }
            _ => None,
        };
        let away_message = matches!(matcher, ResponseMatcher::Away(true))
            .then(|| message.as_ref().param(0).map(str::to_owned))
            .flatten();
        if self.caps.enabled().contains_key(&cap::LABELED_RESPONSE) {
            let track_sync = matches!(
                matcher,
                ResponseMatcher::Join(_) | ResponseMatcher::ListMode { .. }
            );
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
                    started: None,
                    matcher,
                    fence: None,
                    deferred: None,
                    lines: Vec::new(),
                });
            }
            if let Some(channel) = who_sweep {
                self.state.track_who(&channel);
            }
            return Ok(id);
        }
        self.rate_limit.advance(now);
        if self.unlabelled_requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        let line = message.encode()?;
        let id = RequestId(self.next_request_id);
        let who_fence = matches!(&matcher, ResponseMatcher::Who(_))
            .then(|| {
                self.unlabelled_requests
                    .iter_mut()
                    .rev()
                    .find(|request| matches!(&request.matcher, ResponseMatcher::Who(_)))
            })
            .flatten()
            .map(|pending| {
                let token = format!("papo-who-{}", self.next_request_id);
                debug_assert!(pending.fence.is_none());
                pending.fence = Some(token.clone());
                (pending.id, token, pending.deferred.is_none())
            });
        let fence = matches!(matcher, ResponseMatcher::Mode(_))
            .then(|| format!("papo-mode-{}", self.next_request_id));
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if let Some((request, token, true)) = &who_fence {
            let ping = Line::new("PING").param(token.as_str()).encode()?;
            self.outbox.transmit_ordered(ping, Some(*request));
        }
        let deferred = who_fence.is_some().then(|| line.clone());
        self.unlabelled_requests.push_back(UnlabelledRequest {
            id,
            started: None,
            matcher,
            fence: fence.clone(),
            deferred,
            lines: Vec::new(),
        });
        if let Some(message) = away_message {
            self.away_messages.insert(id, message);
        }
        if who_fence.is_none() {
            if let Some(fence) = fence {
                self.outbox.transmit_raw(line);
                let ping = Line::new("PING").param(fence).encode()?;
                self.outbox.transmit_tracked(ping, None, Some(id));
            } else {
                self.outbox.transmit_tracked(line, None, Some(id));
            }
        }
        if let Some(channel) = who_sweep {
            self.state.track_who(&channel);
        }
        Ok(id)
    }

    /// Authenticates as an IRC operator without copying the password into a plain line.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::InvalidSecret`] for a byte secret. Other failures are the same as
    /// [`Session::request_with_matcher`].
    pub fn oper(
        &mut self,
        now: Instant,
        name: &str,
        password: &Secret,
    ) -> Result<RequestId, RequestError> {
        if password.expose().is_none() {
            return Err(RequestError::InvalidSecret);
        }
        self.request_with_matcher(
            now,
            Line::new("OPER").param(name).param(password),
            ResponseMatcher::Oper,
        )
    }

    /// Joins a channel, optionally using a secret channel key.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::InvalidSecret`] for a byte key. Other failures are the same as
    /// [`Session::request_with_matcher`].
    pub fn join(
        &mut self,
        now: Instant,
        channel: &str,
        key: Option<&Secret>,
    ) -> Result<RequestId, RequestError> {
        if key.is_some_and(|key| key.expose().is_none()) {
            return Err(RequestError::InvalidSecret);
        }
        let mut line = Line::new("JOIN").param(channel);
        if let Some(key) = key {
            line = line.param(key);
        }
        self.request_with_matcher(now, line, ResponseMatcher::Join(channel.into()))
    }

    /// Adds nicknames to the desired server-side monitor list.
    pub fn monitor_add<I, S>(&mut self, nicks: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let before = self.state.change_generation();
        self.monitor.add(
            nicks,
            self.isupport.value(),
            self.welcome_state.is_connected(),
            &mut self.outbox,
        );
        self.sync_monitored_state();
        if before != self.state.change_generation() {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    /// Removes nicknames from the desired server-side monitor list.
    pub fn monitor_remove<I, S>(&mut self, nicks: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let before = self.state.change_generation();
        self.monitor.remove(
            nicks,
            self.isupport.value(),
            self.welcome_state.is_connected(),
            &mut self.outbox,
        );
        self.sync_monitored_state();
        if before != self.state.change_generation() {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    /// Clears the desired server-side monitor list.
    pub fn monitor_clear(&mut self) {
        let before = self.state.change_generation();
        let supported =
            self.welcome_state.is_connected() && self.isupport.value().monitor().is_some();
        self.monitor.clear(supported, &mut self.outbox);
        self.sync_monitored_state();
        if before != self.state.change_generation() {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
    }

    /// Returns the next CRLF-terminated wire line to transmit.
    pub fn poll_transmit(&mut self) -> Option<WireLine> {
        self.poll_transmit_tracked()
            .map(|transmit| transmit.into_parts().0)
    }

    /// Returns the next wire line with its optional write-completion identifier.
    #[doc(hidden)]
    pub fn poll_transmit_tracked(&mut self) -> Option<Transmit> {
        let batch_open = matches!(
            self.outbox.transmits.front(),
            Some(Transmission::Batch { started: true, .. })
        );
        if !batch_open && let Some(line) = self.outbox.immediate.pop_front() {
            self.mark_request_started(line.request);
            return Some(Transmit {
                line: line.line,
                token: line.token,
            });
        }
        if self.outbox.transmits.is_empty()
            || !self.rate_limit.take(self.isupport.value().saferate())
        {
            return None;
        }
        match self.outbox.transmits.pop_front()? {
            Transmission::Line(line) => {
                self.mark_request_started(line.request);
                Some(Transmit {
                    line: line.line,
                    token: line.token,
                })
            }
            Transmission::Batch {
                mut lines,
                token,
                request,
                ..
            } => {
                let line = lines.pop_front();
                let last = lines.is_empty();
                if !lines.is_empty() {
                    self.outbox.transmits.push_front(Transmission::Batch {
                        lines,
                        started: true,
                        token,
                        request,
                    });
                }
                if last {
                    self.mark_request_started(request);
                }
                line.map(|line| Transmit {
                    line,
                    token: last.then_some(token).flatten(),
                })
            }
        }
    }

    fn mark_request_started(&mut self, request: Option<RequestId>) {
        let (Some(id), Some(now)) = (request, self.rate_limit.now) else {
            return;
        };
        if let Some((_, started, _, _, _)) = self
            .requests
            .values_mut()
            .find(|(request_id, started, _, _, _)| *request_id == id && started.is_none())
        {
            *started = Some(now);
        }
        for pending in &mut self.unlabelled_requests {
            if pending.id == id && pending.started.is_none() {
                pending.started = Some(now);
            }
        }
    }

    /// Returns the next session event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.outbox.events.pop_front()
    }

    /// Returns the next connection phase reported by protocol processing.
    ///
    /// Transport shells supply [`ConnectPhase::Transport`] and [`ConnectPhase::Tls`].
    pub fn poll_connect_phase(&mut self) -> Option<ConnectPhase> {
        self.outbox.connect_phases.pop_front()
    }

    /// Returns the next protocol deadline, if any.
    #[must_use]
    pub fn poll_timeout(&self) -> Option<Instant> {
        [
            self.welcome_state.next_deadline(),
            self.sasl.next_deadline(),
            self.keepalive.next_deadline(),
            self.rate_limit.next_deadline(
                !self.outbox.transmits.is_empty(),
                self.isupport.value().saferate(),
            ),
            self.batches.next_deadline(),
            self.state.typing.next_deadline(),
            self.nick_regain.next_deadline(),
            self.requests
                .values()
                .filter_map(|(_, started, _, _, _)| {
                    started.and_then(|started| started.checked_add(self.config.request_timeout))
                })
                .min(),
            self.unlabelled_requests
                .iter()
                .filter_map(|request| {
                    request
                        .started
                        .and_then(|started| started.checked_add(self.config.request_timeout))
                })
                .min(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Returns the nickname currently assigned to this session.
    #[must_use]
    pub fn nick(&self) -> &str {
        self.welcome_state.nick()
    }

    /// Returns the `IRCv3` capabilities enabled on this connection.
    ///
    /// Look up a capability with [`&Cap`](cap::Cap), using either its draft or final spelling, to
    /// treat aliases as the same capability. A plain `&str` lookup only matches the spelling the
    /// server used.
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

    /// Returns a counter that changes when visible typing state changes.
    #[doc(hidden)]
    #[must_use]
    pub const fn typing_generation(&self) -> u64 {
        self.typing_generation
    }

    /// Returns the identifier that the next tracked request will receive.
    #[doc(hidden)]
    #[must_use]
    pub const fn next_request_id(&self) -> RequestId {
        RequestId(self.next_request_id)
    }

    fn reserve_request_id(&mut self) -> RequestId {
        let id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.wrapping_add(1);
        id
    }

    /// Reports whether the connection is ready for use.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.welcome_state.is_connected()
    }

    /// Returns the account name reported by the server, when authenticated.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        match self.state.account() {
            Account::LoggedIn(account) => Some(account.as_ref()),
            Account::Unknown | Account::LoggedOut => None,
        }
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
    pub fn create_account(
        &mut self,
        now: Instant,
        account: Option<&str>,
        email: Option<&str>,
        password: impl Into<Secret>,
    ) -> Result<(), RequestError> {
        if self.connection.security == ConnectionSecurity::Plaintext
            && !self.config.allow_insecure_auth
        {
            return Err(RequestError::InsecureAuthentication);
        }
        let registered = self.welcome_state.is_connected();
        let value = if registered {
            self.caps
                .enabled()
                .get(&cap::ACCOUNT_REGISTRATION)
                .map(Option::as_deref)
        } else {
            self.caps
                .advertised_value(cap::ACCOUNT_REGISTRATION.as_str())
                .map(Option::as_deref)
        };
        let Some(value) = value else {
            return self.require_feature(Feature::AccountCreation);
        };
        let policy = RegistrationPolicy::parse(value);
        if !registered && !policy.before_connect {
            return self.require_feature(Feature::AccountCreation);
        }
        if email.is_none() && policy.email_required {
            return Err(RequestError::EmailRequired);
        }
        if account.is_some_and(|account| {
            !policy.custom_account_name
                && !names_equal(self.isupport.value().casemapping(), account, self.nick())
        }) {
            return Err(RequestError::CustomAccountNameUnsupported);
        }
        let password = password.into();
        if !registered {
            if !self.caps.is_pending(cap::ACCOUNT_REGISTRATION.as_str()) {
                return self.require_feature(Feature::AccountCreation);
            }
            let request = self.reserve_request_id();
            self.account_registration.begin(PendingRegistration {
                request: Some(request),
                account: account.map(str::to_owned),
                email: email.map(str::to_owned),
                password,
            });
            return Ok(());
        }
        self.send_register(now, account, email, &password, None)
    }

    fn send_register(
        &mut self,
        now: Instant,
        account: Option<&str>,
        email: Option<&str>,
        password: &Secret,
        request: Option<RequestId>,
    ) -> Result<(), RequestError> {
        if password.expose().is_none() {
            return Err(RequestError::InvalidSecret);
        }
        let message = Line::new("REGISTER")
            .param(account.unwrap_or("*"))
            .param(email.unwrap_or("*"))
            .param(password);
        if let Some(request) = request {
            self.require_feature(Feature::AccountCreation)?;
            self.queue_reserved_request(now, message, Feature::AccountCreation, request)?;
        } else {
            self.request_for_feature(now, message, Feature::AccountCreation)?;
        }
        self.account_registration.request_started();
        Ok(())
    }

    /// Submits a verification code for `draft/account-registration`.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when the capability was not negotiated, or
    /// [`RequestError::InvalidSecret`] when `code` contains bytes rather than text. Other failures
    /// are the same as [`Session::request`].
    pub fn verify(
        &mut self,
        now: Instant,
        account: &str,
        code: impl Into<Secret>,
    ) -> Result<(), RequestError> {
        let code = code.into();
        if code.expose().is_none() {
            return Err(RequestError::InvalidSecret);
        }
        self.require_feature(Feature::AccountCreation)?;
        if !self.welcome_state.is_connected()
            && !RegistrationPolicy::parse(
                self.caps
                    .enabled()
                    .get(&cap::ACCOUNT_REGISTRATION)
                    .and_then(Option::as_deref),
            )
            .before_connect
        {
            return Err(RequestError::unsupported(Feature::AccountCreation));
        }
        self.request_for_feature(
            now,
            Line::new("VERIFY").param(account).param(code),
            Feature::AccountCreation,
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
        let available = self.caps.enabled().contains_key(&cap::SASL);
        self.sasl.reauthenticate(
            now,
            self.connection.security,
            available,
            self.caps
                .enabled()
                .get(&cap::SASL)
                .and_then(Option::as_deref),
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
        let pre_away = self.caps.enabled().contains_key(&cap::PRE_AWAY);
        if !pre_away && !self.welcome_state.is_connected() {
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

    #[allow(clippy::too_many_lines)]
    fn dispatch_message(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        context: &DispatchContext,
        emit_unhandled: bool,
        enclosing_request: Option<RequestId>,
    ) {
        let queued_events = self.outbox.events.len();
        let emit_response_events = self.response_emits_events(message);
        let before_state_change = self.state.change_generation();
        let before_isupport_generation = self.isupport.value().__generation();
        let before_typing = self.state.__typing_table().clone();
        let was_connected = self.welcome_state.is_connected();
        let before_features = was_connected.then(|| self.supported_features());
        let registration_handled = self.welcome_state.handle_message(
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
        self.welcome_state
            .handle_cap_decision(message, &self.config, &mut self.outbox);
        let sasl_handled = self.sasl.handle_message(now, message, &mut self.outbox) == Handled::Yes;
        let account_registration_handled = self.account_registration.handle_message(
            message,
            self.isupport.value().casemapping(),
            &mut self.outbox,
        ) == Handled::Yes;
        if sasl_handled || account_registration_handled {
            let account = self
                .account_registration
                .account()
                .or_else(|| self.sasl.account())
                .map(str::to_owned);
            self.state.sync_own_account(account.as_deref());
        }
        self.maybe_send_away_on_connect();
        let isupport_handled =
            self.isupport.handle_message(message, &mut self.outbox) == Handled::Yes;
        if isupport_handled {
            self.remember_casemapping();
        }
        let connection_finished = !was_connected && self.welcome_state.is_connected();
        if connection_finished {
            self.monitor
                .restore(self.isupport.value(), &mut self.outbox);
            self.nick_regain.start(
                now,
                &self.config,
                self.isupport.value(),
                &self.monitor,
                &mut self.outbox,
            );
        }
        let monitor_handled = self.monitor.handle_message(
            message,
            self.isupport.value(),
            self.nick_regain.monitored_nick(&self.config.nick),
            &mut self.outbox,
        ) == Handled::Yes;
        self.send_pending_registration(now);
        if connection_finished || isupport_handled {
            self.sync_monitored_state();
        } else if monitor_handled {
            for nick in monitor::presence_nicks(message) {
                self.state
                    .sync_monitored_nick(&self.monitor, self.isupport.value(), nick);
            }
        }
        let whoami = self.handle_whoami(message);
        let own_away_message = self.confirmed_away_message(message).map(str::to_owned);
        let typing_handled = self.report_typing(now, message, context.batch_type().is_some());
        self.stop_typing_for_message(message);
        self.stop_typing_for_departure(message);
        self.carry_typing_for_nick(message);
        let state_handled = whoami
            || self.state.handle_message(
                message,
                self.welcome_state.nick(),
                self.isupport.value(),
                self.caps.enabled(),
                self.no_implicit_names(),
                MetadataContext {
                    max_keys: state::MAX_METADATA_KEYS_PER_TARGET,
                    subscription_batch: context.batch_type() == Some("metadata-subs"),
                },
                own_away_message.as_deref(),
                &mut self.outbox,
            ) == Handled::Yes;
        let nick_regain_handled = self.nick_regain.handle_message(
            now,
            message,
            self.welcome_state.is_connected(),
            self.state.nick(),
            &self.config,
            self.isupport.value(),
            &self.monitor,
            &mut self.outbox,
        );
        self.handle_ctcp(now, message, context);
        let handled = registration_handled
            || caps_handled
            || sasl_handled
            || account_registration_handled
            || nick_regain_handled
            || monitor_handled
            || typing_handled
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
        let labelled_request = enclosing_request.or_else(|| {
            message.tag("label").and_then(|label| {
                self.requests
                    .get(label.as_ref())
                    .map(|(id, _, _, _, _)| *id)
            })
        });
        let labelled_response =
            self.resolve_labelled_message(message, queued_events, emit_response_events);
        let response = self.resolve_unlabelled(now, message, labelled_request) || labelled_response;
        if response && labelled_echo && emit_response_events {
            self.enqueue_labelled_echo(message);
        }
        let tag_message =
            !response && !typing_handled && emit_unhandled && self.report_tag_message(message);
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
        self.caps.maybe_finish(
            self.sasl.is_finished() && self.account_registration.authentication_finished(),
            self.config.selected_bouncer_network(),
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
            let received =
                self.received_line(message.to_owned(), context.batch_type().map(str::to_owned));
            self.outbox.event(Event::Line(received));
        }
        if message.command() == Command::Nick
            && self.welcome_state.nick() != self.state.nick().as_ref()
        {
            self.welcome_state.update_nick(self.state.nick());
        }
        self.apply_close_request();
        self.report_feature_changes(before_features);
        let remembers_message = matches!(message.command(), Command::Privmsg | Command::Notice)
            && message.tag("msgid").is_some()
            && message.tag("time").is_some();
        if caps_handled
            || sasl_handled
            || account_registration_handled
            || (isupport_handled
                && before_isupport_generation != self.isupport.value().__generation())
            || monitor_handled
            || whoami
            || state_handled
            || remembers_message
            || before_state_change != self.state.change_generation()
        {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
        if !before_typing.same_entries(self.state.__typing_table()) {
            self.typing_generation = self.typing_generation.wrapping_add(1);
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

    fn send_pending_registration(&mut self, now: Instant) {
        if self.caps.enabled().contains_key(&cap::ACCOUNT_REGISTRATION) {
            if let Some(pending) = self.account_registration.take_pending() {
                self.config.registration_on_connect = None;
                _ = self.send_register(
                    now,
                    pending.account.as_deref(),
                    pending.email.as_deref(),
                    &pending.password,
                    pending.request,
                );
            }
        } else if self.caps.is_negotiating()
            && !self.caps.is_pending(cap::ACCOUNT_REGISTRATION.as_str())
        {
            self.account_registration.cancel_pending();
        }
    }

    fn handle_whoami(&mut self, message: &LineRef<'_>) -> bool {
        if !self.caps.enabled().contains_key(&cap::WHOAMI)
            || !self.welcome_state.is_in_welcome_burst()
            || !self.state.handle_whoami(
                message,
                self.welcome_state.nick(),
                self.isupport.value(),
                &mut self.outbox,
            )
        {
            return false;
        }
        if let Some(ProtoSource::User { nick, .. }) = message.source() {
            self.welcome_state.update_nick(nick);
        }
        true
    }

    fn handle_ctcp(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        context: &DispatchContext,
    ) -> bool {
        if context
            .batch_type()
            .is_some_and(|kind| !matches!(kind, "netsplit" | "netjoin"))
        {
            return false;
        }
        let (Some(source @ ProtoSource::User { nick, .. }), Some(text)) =
            (message.source(), message.param(1))
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
        if self.is_own_source(message) {
            return true;
        }
        let live_query = matches!(context.batch_type(), None | Some("netsplit" | "netjoin"));
        let private_to_us = message.param(0).is_some_and(|target| {
            names_equal(self.isupport.value().casemapping(), target, self.nick())
        });
        if !live_query || !private_to_us {
            return true;
        }
        let response = match query.command.to_ascii_uppercase().as_str() {
            "VERSION" => self.config.ctcp.version.as_deref().map(Some),
            "PING" if self.config.ctcp.ping => Some(query.params),
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
        self.ctcp_rate_limit.advance(now);
        if !self.ctcp_rate_limit.take(false) {
            return true;
        }
        if self.ctcp_sources.len() == MAX_CTCP_SOURCES {
            self.ctcp_sources.pop_front();
        }
        self.ctcp_sources.push_back((nick.to_owned(), now));
        let actor = Source::from_proto(source, self.isupport.value().casemapping());
        if let Some(reply) = __ctcp_reply(&actor, query.command, response.unwrap_or_default())
            && let Ok(line) = reply.encode()
        {
            self.outbox.transmit_raw(line);
        }
        true
    }

    fn dispatch_batch_event(&mut self, now: Instant, event: BatchEvent) {
        match event {
            event @ BatchEvent::Response { .. } => self.dispatch_response_batch(now, event),
            BatchEvent::History {
                target,
                mut items,
                end,
            } => {
                if let Some(conversation) = self.history_conversation(&target) {
                    self.mark_history_authorship(&mut items);
                    self.outbox.event(Event::History {
                        conversation,
                        items,
                        end,
                    });
                }
            }
            BatchEvent::HistoryTargets(targets) => {
                self.outbox
                    .event(Event::HistoryTargets(self.history_targets(targets)));
            }
            BatchEvent::Netsplit { servers, quits } => {
                let authorship = quits
                    .iter()
                    .map(|message| self.authorship(&message.as_ref()))
                    .collect();
                // History can describe stale membership, while netsplit and netjoin batches
                // report connectivity changes that define the current live membership.
                let channels = quits
                    .iter()
                    .map(|message| self.apply_aggregate_state(message).unwrap_or_default())
                    .collect();
                self.outbox.event(Event::Netsplit {
                    servers,
                    quits,
                    authorship,
                    channels,
                });
            }
            BatchEvent::Netjoin { servers, joins } => {
                let authorship = joins
                    .iter()
                    .map(|message| self.authorship(&message.as_ref()))
                    .collect();
                for message in &joins {
                    self.apply_aggregate_state(message);
                }
                self.outbox.event(Event::Netjoin {
                    servers,
                    joins,
                    authorship,
                });
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
            BatchEvent::Line { line, batch_type } => {
                let context = batch_type
                    .clone()
                    .map_or_else(DispatchContext::default, DispatchContext::from_batch_type);
                self.handle_ctcp(now, &line.as_ref(), &context);
                let received = self.received_line(line, batch_type);
                self.outbox.event(Event::Line(received));
            }
            BatchEvent::Multiline(message) => {
                let message = self.received_line(message, None);
                self.outbox
                    .event(Event::MultilineMessage(MultilineMessage { message }));
            }
            BatchEvent::Metadata { kind, messages } => {
                if kind == "metadata-subs" {
                    self.state.clear_metadata_subscriptions();
                }
                let context = DispatchContext::from_batch_type(kind);
                for message in messages {
                    self.dispatch_message(now, &message.as_ref(), &context, true, None);
                }
            }
            #[cfg(feature = "unstable")]
            BatchEvent::Bouncer(messages) => {
                let context = DispatchContext::from_batch_type(cap::BOUNCER_NETWORKS.into());
                for message in messages {
                    self.dispatch_message(now, &message.as_ref(), &context, true, None);
                }
            }
            BatchEvent::Isupport(messages) => {
                let before = self.isupport.value().__generation();
                self.isupport.handle_batch(&messages, &mut self.outbox);
                self.remember_casemapping();
                self.sync_monitored_state();
                if before != self.isupport.value().__generation() {
                    self.state_generation = self.state_generation.wrapping_add(1);
                }
            }
        }
    }

    fn history_conversation(&self, target: &str) -> Option<Conversation> {
        let (status, conversation) = self.state.parse_target(target).ok()?;
        (status.is_none()
            && matches!(
                conversation,
                Conversation::Channel(_) | Conversation::Private(_)
            ))
        .then_some(conversation)
    }

    fn mark_history_authorship(&self, items: &mut [HistoryItem]) {
        for item in items {
            let subject_authorship = match item {
                HistoryItem::ModeChanged(line)
                    if line
                        .line
                        .as_ref()
                        .param(0)
                        .is_some_and(|target| self.isupport.value().is_channel(target)) =>
                {
                    Authorship::Other
                }
                _ => Authorship::Unknown,
            };
            let line = item.line_mut();
            line.authorship = self.history_authorship(&line.line.as_ref());
            line.subject_authorship = subject_authorship;
        }
    }

    fn history_authorship(&self, message: &LineRef<'_>) -> Authorship {
        let Some(papo_proto::Source::User { .. }) = message.source() else {
            return Authorship::Unknown;
        };
        let (Some(actor), Some(own)) = (message.tag("account"), self.account()) else {
            return Authorship::Unknown;
        };
        let mapping = self.isupport.value().casemapping();
        if mapping.account(actor.as_ref()) == mapping.account(own) {
            Authorship::Own
        } else {
            Authorship::Other
        }
    }

    pub(crate) fn finish_mode_list_response(
        &mut self,
        matcher: &ResponseMatcher,
        response: &mut Response,
    ) {
        let ResponseMatcher::ListMode { channel, mode } = matcher else {
            return;
        };
        let mapping = self.isupport.value().casemapping();
        let row = mode.numerics().0;
        let entries: Vec<_> = response
            .lines
            .iter()
            .filter_map(|line| {
                let line = line.as_ref();
                let Command::Numeric(numeric) = line.command() else {
                    return None;
                };
                if numeric.code() != row {
                    return None;
                }
                let offset = usize::from(*mode == ListMode::Quiet);
                if offset == 1 && line.param(2) != Some("q") {
                    return None;
                }
                Some(ModeListEntry {
                    mask: mapping.mask(line.param(2 + offset)?),
                    actor: line
                        .param(3 + offset)
                        .map(|actor| Source::from_prefix(actor, mapping)),
                    set_at: line
                        .param(4 + offset)
                        .and_then(|value| value.parse().ok())
                        .and_then(|seconds| {
                            std::time::UNIX_EPOCH
                                .checked_add(std::time::Duration::from_secs(seconds))
                        }),
                })
            })
            .collect();
        response.mode_list = Some(entries.clone());
        let bouncer = self.caps.bouncer_enabled()
            || !matches!(
                self.isupport.value().get("BOUNCER_NETID"),
                papo_proto::Advertised::Absent
            );
        if response.error().is_some() || bouncer {
            self.state.invalidate_mode_list(channel, *mode);
        } else {
            self.state.set_mode_list(channel, *mode, entries);
        }
    }

    fn history_targets(
        &self,
        targets: Vec<(String, std::time::SystemTime)>,
    ) -> Vec<(Conversation, std::time::SystemTime)> {
        targets
            .into_iter()
            .filter_map(|(target, latest)| Some((self.history_conversation(&target)?, latest)))
            .collect()
    }

    fn remember_casemapping(&mut self) {
        self.state.update_isupport(self.isupport.value());
        let new = self.isupport.value().casemapping();
        if !self.casemapping_change_emitted && new != self.casemapping_baseline {
            self.outbox.event(Event::CasemappingChanged {
                old: self.casemapping_baseline,
                new,
            });
            self.casemapping_change_emitted = true;
        }
        self.remembered_casemapping = new;
    }

    fn dispatch_response_batch(&mut self, now: Instant, response: BatchEvent) {
        let BatchEvent::Response {
            label,
            echo,
            messages,
            batch_type,
            mut history,
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
        let bouncer_batch = cfg!(feature = "unstable") && batch_type == cap::BOUNCER_NETWORKS_NAME;
        let context = DispatchContext::from_batch_type(batch_type);
        if history.is_none() && !bouncer_batch {
            for message in &messages {
                self.dispatch_message(
                    now,
                    &message.as_ref(),
                    &context,
                    request.is_none(),
                    request.as_ref().map(|(id, _, _, _, _)| *id),
                );
            }
        }
        if !emit_events {
            self.outbox.events.truncate(queued_events);
        }
        if let Some((_, items, _)) = &mut history {
            self.mark_history_authorship(items);
        }
        if deferred && let Some((id, _, _, _, _)) = &request {
            self.resolve_deferred(*id);
        }
        if let Some((id, _, sent, _, _)) = request
            && !deferred
        {
            let whois_channels = parse_whois_channels(&messages, self.isupport.value());
            let enabled_features = self.supported_features();
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
                history: history.and_then(|(target, items, end)| {
                    Some(HistoryPage {
                        conversation: self.history_conversation(&target)?,
                        items,
                        end,
                    })
                }),
                history_targets: history_targets.map(|targets| self.history_targets(targets)),
                whois_channels,
                mode_list: None,
                state_limit: None,
                enabled_features,
            }));
            for message in echoed.into_iter().flatten() {
                let received = self.received_line(message, None);
                self.outbox.event(Event::Line(received));
            }
        }
    }

    fn apply_aggregate_state(&mut self, message: &Line) -> Option<Vec<ChannelName>> {
        let event_count = self.outbox.events.len();
        let handled = self.state.handle_message(
            &message.as_ref(),
            self.welcome_state.nick(),
            self.isupport.value(),
            self.caps.enabled(),
            self.no_implicit_names(),
            MetadataContext {
                max_keys: state::MAX_METADATA_KEYS_PER_TARGET,
                subscription_batch: false,
            },
            None,
            &mut self.outbox,
        ) == Handled::Yes;
        let generated = self.outbox.events.split_off(event_count);
        let channels = generated.iter().find_map(|event| match event {
            Event::Quit { channels, .. } => Some(channels.clone()),
            _ => None,
        });
        self.outbox.events.extend(
            generated
                .into_iter()
                .filter(|event| matches!(event, Event::StateLimitReached { .. })),
        );
        if handled {
            self.state_generation = self.state_generation.wrapping_add(1);
        }
        channels
    }

    fn no_implicit_names(&self) -> bool {
        self.caps.enabled().keys().any(|name| {
            cap::canonical(name.as_str()) == cap::canonical(cap::NO_IMPLICIT_NAMES.as_str())
        })
    }

    fn maybe_subscribe_metadata(&mut self, now: Instant) {
        if self.metadata_subscriptions_sent || self.config.metadata_subscriptions.is_empty() {
            return;
        }
        let Some(limits) = self.metadata_limits() else {
            return;
        };
        let registered = self.welcome_state.is_connected();
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

fn requested_message_feature(message: &LineRef<'_>) -> Option<Feature> {
    if message.command_is("REDACT") {
        return Some(Feature::Redaction);
    }
    if message.tag("+draft/react").is_some() || message.tag("+draft/unreact").is_some() {
        return Some(Feature::Reactions);
    }
    message.tag("+typing").map(|_| Feature::Typing)
}

fn parse_whois_channels(lines: &[Line], isupport: &papo_proto::Isupport) -> Vec<WhoisChannel> {
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

fn sent_info(message: &LineRef<'_>) -> Option<SentInfo> {
    Some(SentInfo {
        msgid: message.tag("msgid")?.into_owned(),
        server_time: message.tag("time").map(std::borrow::Cow::into_owned),
    })
}

fn text_label(prefix: Option<&str>, index: usize) -> Option<String> {
    prefix.map(|prefix| {
        if index == 0 {
            prefix.to_owned()
        } else {
            format!("{prefix}-{index}")
        }
    })
}

#[cfg(test)]
mod tests {
    use papo_proto::{Casemapping, Line, LineRef};

    use super::{
        CloseKind, Config, Event, ListMode, Outbox, RequestId, ResponseMatcher, Session, State,
        UnlabelledRequest,
    };

    #[test]
    fn invalidating_a_list_request_uses_the_requested_id() {
        let now = std::time::Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b":papo!~papo@localhost JOIN #one");
        session.handle_line(now, b":papo!~papo@localhost JOIN #two");
        session.unlabelled_requests.clear();
        session
            .state
            .set_mode_list("#one", ListMode::Ban, Vec::new());
        session
            .state
            .set_mode_list("#two", ListMode::Quiet, Vec::new());
        let requested = RequestId(41);
        session.unlabelled_requests.extend([
            UnlabelledRequest {
                id: requested,
                started: None,
                matcher: ResponseMatcher::ListMode {
                    channel: "#one".into(),
                    mode: ListMode::Ban,
                },
                fence: None,
                deferred: None,
                lines: Vec::new(),
            },
            UnlabelledRequest {
                id: RequestId(42),
                started: None,
                matcher: ResponseMatcher::ListMode {
                    channel: "#two".into(),
                    mode: ListMode::Quiet,
                },
                fence: None,
                deferred: None,
                lines: Vec::new(),
            },
        ]);

        session.invalidate_list_request(requested);

        assert_eq!(
            session.state().channel("#one").unwrap().list(ListMode::Ban),
            None
        );
        assert_eq!(
            session
                .state()
                .channel("#two")
                .unwrap()
                .list(ListMode::Quiet),
            Some([].as_slice())
        );
    }

    #[test]
    fn ordinary_messages_do_not_clone_full_state() {
        let now = std::time::Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
        session.handle_line(
            now,
            b":ergo.test 353 papo = #papo :papo alice emily jordan riley",
        );
        super::state::reset_clone_count();

        session.handle_line(
            now,
            b"@time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120004 :alice!~alice@localhost PRIVMSG #papo :review is ready",
        );

        assert_eq!(super::state::clone_count(), 0);
    }

    #[test]
    fn monitor_updates_do_not_clone_full_state() {
        let mut session = Session::new(Config::default());
        super::state::reset_clone_count();

        session.monitor_add(["alice"]);
        session.monitor_remove(["alice"]);
        session.monitor_clear();

        assert_eq!(super::state::clone_count(), 0);
    }

    #[test]
    fn aggregate_membership_does_not_clone_full_state() {
        let now = std::time::Instant::now();
        let mut session = Session::new(Config::default());
        session.handle_line(now, b":papo!~papo@localhost JOIN #papo");
        session.handle_line(now, b":alice!~alice@localhost JOIN #papo");
        super::state::reset_clone_count();

        session.handle_line(now, b":ergo.test BATCH +split netsplit irc.a irc.b");
        session.handle_line(
            now,
            b"@batch=split :alice!~alice@localhost QUIT :irc.a irc.b",
        );
        session.handle_line(now, b":ergo.test BATCH -split");

        assert_eq!(super::state::clone_count(), 0);
    }

    #[test]
    fn isupport_batch_does_not_clone_full_state() {
        let now = std::time::Instant::now();
        let mut session = Session::new(Config::default());
        super::state::reset_clone_count();

        session.handle_line(now, b":ergo.test BATCH +support draft/isupport");
        session.handle_line(
            now,
            b"@batch=support :ergo.test 005 papo CASEMAPPING=ascii :are supported",
        );
        session.handle_line(now, b":ergo.test BATCH -support");

        assert_eq!(super::state::clone_count(), 0);
    }

    #[test]
    fn list_mode_matchers_reject_wrong_numeric_target_and_quiet_marker() {
        let mapping = Casemapping::Rfc1459;
        let state = State::new(&Config::default());
        let ban = ResponseMatcher::ListMode {
            channel: "#Papo[".into(),
            mode: ListMode::Ban,
        };
        let quiet = ResponseMatcher::ListMode {
            channel: "#Papo[".into(),
            mode: ListMode::Quiet,
        };
        let line = |wire| LineRef::parse(wire).unwrap();

        let ban_row = line(":ergo.test 367 papo #papo{ *!*@gateway.example oper 1761048000");
        assert!(ban.is_list_row(&ban_row, mapping));
        assert_eq!(ban.matches(&ban_row, mapping, "papo", &state), Some(false));
        for wire in [
            ":ergo.test 348 papo #papo{ *!*@staff.example oper 1761048000",
            ":ergo.test 367 papo #other *!*@gateway.example oper 1761048000",
            ":ergo.test 482 papo #other :You're not a channel operator",
        ] {
            let message = line(wire);
            assert!(!ban.is_list_row(&message, mapping), "accepted {wire}");
            assert_eq!(
                ban.matches(&message, mapping, "papo", &state),
                None,
                "accepted {wire}"
            );
        }

        let wrong_quiet =
            line(":solanum.example 728 papo #papo{ b *!*@noisy.example oper 1761048000");
        assert!(!quiet.is_list_row(&wrong_quiet, mapping));
        assert_eq!(quiet.matches(&wrong_quiet, mapping, "papo", &state), None);
        let quiet_end = line(":solanum.example 729 papo #papo{ q :End of channel quiet list");
        assert!(!quiet.is_list_row(&quiet_end, mapping));
        assert_eq!(
            quiet.matches(&quiet_end, mapping, "papo", &state),
            Some(true)
        );
        assert!(!ResponseMatcher::Ping("token".into()).is_list_row(&ban_row, mapping));
    }

    #[test]
    fn batch_queue_rejects_empty_and_closed_transmissions() {
        let mut outbox = Outbox::default();
        outbox.transmit_tracked_batch(Vec::new(), None, None);
        assert!(outbox.transmits.is_empty());

        outbox.close_requested = true;
        outbox.transmit_tracked_batch(
            vec![
                Line::new("BATCH")
                    .param("+papo")
                    .param("draft/multiline")
                    .param("#papo")
                    .encode()
                    .unwrap(),
            ],
            None,
            None,
        );
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
