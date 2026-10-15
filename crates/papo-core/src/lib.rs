//! Sans-I/O IRC session state and protocol behavior.
//! Time and randomness enter through explicit inputs.

mod caps;
mod framing;
mod isupport;
mod keepalive;
mod ratelimit;
mod registration;
mod sasl;
mod state;
pub mod sts;

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use papo_proto::{
    BuildError, MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message, MessageRef, ModeChange, ModeKind,
};

use caps::Caps;
use framing::Framer;
use isupport::IsupportState;
use keepalive::Keepalive;
pub use ratelimit::RateLimit;
use ratelimit::RateLimiter;
use registration::Registration;
use sasl::SaslSession;
pub use sasl::{Entropy, Sasl};
pub use state::{Account, Channel, Member, State, User};
use sts::ConnectionSecurity;

const MAX_INCOMING_LINE_LEN: usize = MAX_TAG_SECTION_LEN + MAX_MESSAGE_LEN;
const DEFAULT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_PING_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MAX_CHANNELS: usize = 10_000;
const DEFAULT_MAX_MEMBERS_PER_CHANNEL: usize = 100_000;

/// Capabilities requested by a default session when the server advertises them.
pub const DEFAULT_WANTED_CAPS: &[&str] = &[
    "cap-notify",
    "message-tags",
    "server-time",
    "batch",
    "echo-message",
    "account-tag",
    "extended-join",
    "multi-prefix",
    "userhost-in-names",
    "chghost",
    "setname",
    "away-notify",
    "account-notify",
    "invite-notify",
    "standard-replies",
    "labeled-response",
];

/// Enabled `IRCv3` capability names and their advertised values.
pub type Capabilities = BTreeMap<String, Option<String>>;

/// Registration state captured when a session becomes ready for use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Welcome {
    /// The nickname assigned by the server.
    pub nick: String,
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
    pub password: Option<String>,
    /// The maximum time to wait for the welcome numeric.
    pub registration_timeout: Duration,
    /// Idle time after which the client probes the server with `PING`.
    pub ping_interval: Duration,
    /// Maximum silence before the session closes the connection.
    pub ping_timeout: Duration,
    /// Outbound line rate limit.
    pub rate_limit: RateLimit,
    /// Default transport security used by [`Session::handle_connected`].
    pub connection_security: ConnectionSecurity,
    /// Default hostname used by [`Session::handle_connected`].
    pub host: String,
    /// Capability names to request when advertised by the server.
    pub wanted_caps: Vec<String>,
    /// Alternative nicknames to try when registration collides.
    pub alt_nicks: Vec<String>,
    /// SASL mechanisms to try in preference order.
    pub sasl: Option<Sasl>,
    /// Whether registration must stop when SASL is unavailable or fails.
    pub require_sasl: bool,
    /// Ergo multiclient device identifier appended to the authentication identity.
    pub device: Option<String>,
    /// Maximum number of channels retained in session state.
    pub max_channels: usize,
    /// Maximum number of members retained for one channel.
    pub max_members_per_channel: usize,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("decoding", &self.decoding)
            .field("nick", &self.nick)
            .field("username", &self.username)
            .field("realname", &self.realname)
            .field("password", &self.password.as_ref().map(|_| Redacted))
            .field("registration_timeout", &self.registration_timeout)
            .field("ping_interval", &self.ping_interval)
            .field("ping_timeout", &self.ping_timeout)
            .field("rate_limit", &self.rate_limit)
            .field("connection_security", &self.connection_security)
            .field("host", &self.host)
            .field("wanted_caps", &self.wanted_caps)
            .field("alt_nicks", &self.alt_nicks)
            .field("sasl", &self.sasl)
            .field("require_sasl", &self.require_sasl)
            .field("device", &self.device)
            .field("max_channels", &self.max_channels)
            .field("max_members_per_channel", &self.max_members_per_channel)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct Redacted;

impl std::fmt::Debug for Redacted {
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
            alt_nicks: Vec::new(),
            sasl: None,
            require_sasl: false,
            device: None,
            max_channels: DEFAULT_MAX_CHANNELS,
            max_members_per_channel: DEFAULT_MAX_MEMBERS_PER_CHANNEL,
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
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
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

    /// Sets whether registration requires successful SASL authentication.
    #[must_use]
    pub const fn require_sasl(mut self, required: bool) -> Self {
        self.require_sasl = required;
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
    Message(Message),
    /// A line that could not be decoded or parsed.
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// A request for the transport to drop the connection.
    Close {
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

/// A bounded server-controlled state collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StateLimit {
    /// Joined channels.
    Channels,
    /// Members retained for one channel.
    Members,
}

/// Sans-I/O state for one IRC client session.
pub struct Session {
    config: Config,
    connection: Connection,
    caps: Caps,
    framer: Framer,
    isupport: IsupportState,
    keepalive: Keepalive,
    rate_limit: RateLimiter,
    registration: Registration,
    sasl: SaslSession,
    state: State,
    outbox: Outbox,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("config", &self.config)
            .field("connection", &self.connection)
            .field("caps", &self.caps)
            .field("framer", &self.framer)
            .field("isupport", &self.isupport)
            .field("keepalive", &self.keepalive)
            .field("rate_limit", &self.rate_limit)
            .field("registration", &self.registration)
            .field("sasl", &self.sasl)
            .field("state", &self.state)
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
    transmits: VecDeque<Vec<u8>>,
    immediate: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
    close_requested: bool,
}

impl Outbox {
    fn transmit_raw(&mut self, line: Vec<u8>) {
        if self.close_requested {
            return;
        }
        if bypasses_rate_limit(&line) {
            self.immediate.push_back(line);
        } else {
            self.transmits.push_back(line);
        }
    }

    fn transmit<const N: usize>(&mut self, command: &str, params: [&str; N]) {
        let line = Message::new(command, params)
            .to_line()
            .expect("protocol-generated IRC messages are valid");
        self.transmit_raw(line.into());
    }

    fn event(&mut self, event: Event) {
        self.events.push_back(event);
    }

    fn close(&mut self, reason: String) {
        self.close_requested = true;
        self.event(Event::Close { reason });
    }

    fn close_and_discard(&mut self, reason: String) {
        self.transmits.clear();
        self.immediate.clear();
        self.close(reason);
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
        Self {
            config,
            connection,
            caps: Caps::default(),
            framer: Framer::default(),
            isupport,
            keepalive: Keepalive::default(),
            rate_limit,
            registration,
            sasl,
            state,
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
        self.rate_limit.reset(now);
        self.caps.handle_connected(now);
        self.isupport.handle_connected();
        self.sasl.handle_connected();
        self.state.reset(&self.config);
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
            Ok(message) => self.dispatch_message(now, &message),
            Err(_) => self.malformed(line),
        }
    }

    /// Notifies the session that its current deadline was reached.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.rate_limit.advance(now);
        self.registration.handle_timeout(
            now,
            self.caps.enabled(),
            self.isupport.value(),
            &mut self.outbox,
        );
        self.sasl.handle_timeout(now, &mut self.outbox);
        self.caps
            .maybe_finish(self.sasl.is_finished(), &mut self.outbox);
        self.keepalive.handle_timeout(now, &mut self.outbox);
        self.apply_close_request();
    }

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.framer.close();
        self.outbox.reset_connection();
        self.registration.handle_disconnected();
        self.sasl.handle_disconnected();
        self.keepalive.handle_disconnected();
        self.state.reset(&self.config);
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

    /// Returns the next CRLF-terminated wire line to transmit.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        if let Some(line) = self.outbox.immediate.pop_front() {
            return Some(line);
        }
        if self.outbox.transmits.is_empty() || !self.rate_limit.take() {
            return None;
        }
        self.outbox.transmits.pop_front()
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
            self.rate_limit
                .next_deadline(!self.outbox.transmits.is_empty()),
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

    /// Reports whether connection registration has completed.
    #[must_use]
    pub const fn is_registered(&self) -> bool {
        self.registration.is_registered()
    }

    /// Returns the account name reported by the server, when authenticated.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.sasl.account()
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
        self.outbox.transmits.len() + self.outbox.immediate.len()
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

    fn dispatch_message(&mut self, now: Instant, message: &MessageRef<'_>) {
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
        let isupport_handled = self.isupport.handle_message(message) == Handled::Yes;
        let no_implicit_names = self
            .caps
            .enabled()
            .keys()
            .any(|name| papo_proto::canonical(name) == "no-implicit-names");
        let state_handled = self.state.handle_message(
            message,
            self.registration.nick(),
            self.isupport.value(),
            no_implicit_names,
            &mut self.outbox,
        ) == Handled::Yes;
        let handled = registration_handled
            || caps_handled
            || sasl_handled
            || state_handled
            || isupport_handled
            || self
                .keepalive
                .handle_message(now, message, &mut self.outbox)
                == Handled::Yes;
        self.caps
            .maybe_finish(self.sasl.is_finished(), &mut self.outbox);
        if !handled {
            self.outbox.event(Event::Message(message.to_owned()));
        }
        self.apply_close_request();
    }

    fn close_for_long_line(&mut self) {
        self.close("incoming IRC line exceeds 8703 bytes".to_owned());
    }

    fn close(&mut self, reason: String) {
        self.outbox.close(reason);
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
        b"PONG",
    ]
    .iter()
    .any(|exempt| command.eq_ignore_ascii_case(exempt))
}
