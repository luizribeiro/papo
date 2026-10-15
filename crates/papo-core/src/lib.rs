//! Sans-I/O IRC session state and protocol behavior.
//! Time and randomness enter through explicit inputs.

mod batch;
mod caps;
mod framing;
mod isupport;
mod keepalive;
mod monitor;
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

use batch::{BatchAction, BatchEvent, Batches, DispatchContext};
use caps::Caps;
use framing::Framer;
use isupport::IsupportState;
use keepalive::Keepalive;
use monitor::Monitor;
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
const DEFAULT_MAX_OPEN_BATCHES: usize = 64;
const DEFAULT_MAX_BATCH_LINES: usize = 10_000;
const DEFAULT_BATCH_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PENDING_REQUESTS: usize = 256;

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
    "extended-monitor",
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
    /// Maximum number of batches retained at once.
    pub max_open_batches: usize,
    /// Maximum number of server lines retained in one batch.
    pub max_batch_lines: usize,
    /// Maximum time an incomplete batch remains open.
    pub batch_timeout: Duration,
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
            .field("max_open_batches", &self.max_open_batches)
            .field("max_batch_lines", &self.max_batch_lines)
            .field("batch_timeout", &self.batch_timeout)
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
            max_open_batches: DEFAULT_MAX_OPEN_BATCHES,
            max_batch_lines: DEFAULT_MAX_BATCH_LINES,
            batch_timeout: DEFAULT_BATCH_TIMEOUT,
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

/// Completed response to a labelled request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    /// Identifier assigned when the request was queued.
    pub id: RequestId,
    /// Response lines in server delivery order.
    pub lines: Vec<Message>,
}

/// A failure to queue a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RequestError {
    /// The IRC message could not be represented on the wire.
    Build(BuildError),
    /// The session already has 256 labelled requests awaiting replies.
    PendingLimit,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(error) => error.fmt(formatter),
            Self::PendingLimit => formatter.write_str("too many labelled requests are pending"),
        }
    }
}

impl std::error::Error for RequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Build(error) => Some(error),
            Self::PendingLimit => None,
        }
    }
}

impl From<BuildError> for RequestError {
    fn from(error: BuildError) -> Self {
        Self::Build(error)
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
    /// A request was sent without protocol-level response correlation.
    RequestUntracked {
        /// Identifier assigned to the request.
        id: RequestId,
    },
    /// A labelled request received its complete logical response.
    Response(Response),
    /// A completed history or playback batch.
    History {
        /// The history target named by the batch.
        target: String,
        /// Messages in server delivery order.
        messages: Vec<Message>,
        /// Whether the server reported that no further page exists.
        end: bool,
    },
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
    /// A monitored nickname came online or changed its userhost.
    MonitorOnline {
        /// The monitored nickname.
        nick: String,
        /// The user's current username.
        user: String,
        /// The user's current hostname.
        host: String,
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

/// Why an incomplete server batch was released as individual messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BatchAbandonReason {
    /// The server opened more batches than configured.
    OpenBatchLimit,
    /// A batch exceeded its configured retained-line limit.
    LineLimit,
    /// A batch did not close before its deadline.
    Timeout,
}

/// A parsed message annotated with protocol dispatch context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedMessage {
    message: Message,
    batch_type: Option<String>,
    echo: bool,
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
}

impl std::ops::Deref for ReceivedMessage {
    type Target = Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
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
    batches: Batches,
    config: Config,
    connection: Connection,
    caps: Caps,
    framer: Framer,
    isupport: IsupportState,
    keepalive: Keepalive,
    monitor: Monitor,
    rate_limit: RateLimiter,
    requests: BTreeMap<String, (RequestId, Instant)>,
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
            .field("batches", &self.batches)
            .field("config", &self.config)
            .field("connection", &self.connection)
            .field("caps", &self.caps)
            .field("framer", &self.framer)
            .field("isupport", &self.isupport)
            .field("keepalive", &self.keepalive)
            .field("monitor", &self.monitor)
            .field("rate_limit", &self.rate_limit)
            .field("requests", &self.requests)
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
        let batches = Batches::new(&config);
        Self {
            batches,
            config,
            connection,
            caps: Caps::default(),
            framer: Framer::default(),
            isupport,
            keepalive: Keepalive::default(),
            monitor: Monitor::default(),
            rate_limit,
            requests: BTreeMap::new(),
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
        self.requests.clear();
        self.rate_limit.reset(now);
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
            Ok(message) => match self.batches.handle(now, &message) {
                BatchAction::Consumed => {}
                BatchAction::Complete(events) => {
                    for event in events {
                        self.dispatch_batch_event(event);
                    }
                }
                BatchAction::Dispatch(context) => {
                    let before = self.snapshot_values();
                    self.dispatch_message(now, &message, &context);
                    if before != self.snapshot_values() {
                        self.state_generation = self.state_generation.wrapping_add(1);
                    }
                }
            },
            Err(_) => self.malformed(line),
        }
    }

    /// Notifies the session that its current deadline was reached.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.rate_limit.advance(now);
        for event in self.batches.handle_timeout(now) {
            self.dispatch_batch_event(event);
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
        self.caps
            .maybe_finish(self.sasl.is_finished(), &mut self.outbox);
        self.keepalive.handle_timeout(now, &mut self.outbox);
        self.apply_close_request();
    }

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.framer.close();
        self.outbox.reset_connection();
        self.batches.reset();
        self.registration.handle_disconnected();
        self.sasl.handle_disconnected();
        self.keepalive.handle_disconnected();
        self.monitor.handle_disconnected();
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

    /// Queues a message and assigns an identifier for its eventual response.
    ///
    /// When `labeled-response` is enabled, the message receives a fresh `label` tag and remains
    /// pending. Otherwise it is sent unchanged and [`Event::RequestUntracked`] is emitted.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::PendingLimit`] when 256 labelled requests are already pending, or
    /// [`RequestError::Build`] when the tagged message cannot be represented on the wire.
    pub fn request(
        &mut self,
        now: Instant,
        mut message: Message,
    ) -> Result<RequestId, RequestError> {
        self.rate_limit.advance(now);
        let id = RequestId(self.next_request_id);
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
            self.requests.insert(label, (id, now));
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox.transmit_raw(line.into());
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
            self.batches.next_deadline(),
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

    /// Returns a counter that changes when snapshot-visible state changes.
    #[must_use]
    pub const fn state_generation(&self) -> u64 {
        self.state_generation
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

    fn snapshot_values(&self) -> (State, Capabilities, papo_proto::Isupport, Option<String>) {
        (
            self.state.clone(),
            self.caps.enabled().clone(),
            self.isupport.value().clone(),
            self.sasl.account().map(str::to_owned),
        )
    }

    fn dispatch_message(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        context: &DispatchContext,
    ) {
        let was_registered = self.registration.is_registered();
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
        if !was_registered && self.registration.is_registered() {
            self.monitor
                .restore(self.isupport.value(), &mut self.outbox);
        }
        let monitor_handled =
            self.monitor
                .handle_message(message, self.isupport.value(), &mut self.outbox)
                == Handled::Yes;
        self.sync_monitored_state();
        let state_handled = self.state.handle_message(
            message,
            self.registration.nick(),
            self.isupport.value(),
            self.no_implicit_names(),
            &mut self.outbox,
        ) == Handled::Yes;
        let handled = registration_handled
            || caps_handled
            || sasl_handled
            || monitor_handled
            || state_handled
            || isupport_handled
            || self
                .keepalive
                .handle_message(now, message, &mut self.outbox)
                == Handled::Yes;
        let response = self.resolve_message(message);
        self.caps
            .maybe_finish(self.sasl.is_finished(), &mut self.outbox);
        if !handled && !response {
            let received =
                self.received_message(message.to_owned(), context.batch_type().map(str::to_owned));
            self.outbox.event(Event::Message(received));
        }
        self.apply_close_request();
    }

    fn resolve_message(&mut self, message: &MessageRef<'_>) -> bool {
        let Some(label) = message.tag("label") else {
            return false;
        };
        let Some((id, _)) = self.requests.remove(label.as_ref()) else {
            return false;
        };
        let lines = if message.command_is("ACK") {
            Vec::new()
        } else {
            vec![message.to_owned()]
        };
        self.outbox.event(Event::Response(Response { id, lines }));
        true
    }

    fn dispatch_batch_event(&mut self, event: BatchEvent) {
        match event {
            BatchEvent::History {
                target,
                messages,
                end,
            } => self.outbox.event(Event::History {
                target,
                messages,
                end,
            }),
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
                let received = self.received_message(message, batch_type);
                self.outbox.event(Event::Message(received));
            }
        }
    }

    fn received_message(&self, message: Message, batch_type: Option<String>) -> ReceivedMessage {
        let echo = message.as_ref().source().is_some_and(|source| {
            let papo_proto::Source::User { nick, .. } = source else {
                return false;
            };
            self.isupport
                .value()
                .casemapping()
                .eq(nick, self.registration.nick())
        });
        ReceivedMessage {
            message,
            batch_type,
            echo,
        }
    }

    fn apply_aggregate_state(&mut self, message: &Message) {
        let event_count = self.outbox.events.len();
        self.state.handle_message(
            &message.as_ref(),
            self.registration.nick(),
            self.isupport.value(),
            self.no_implicit_names(),
            &mut self.outbox,
        );
        let generated = self.outbox.events.split_off(event_count);
        self.outbox.events.extend(
            generated
                .into_iter()
                .filter(|event| matches!(event, Event::StateLimitReached { .. })),
        );
    }

    fn no_implicit_names(&self) -> bool {
        self.caps
            .enabled()
            .keys()
            .any(|name| papo_proto::canonical(name) == "no-implicit-names")
    }

    fn close_for_long_line(&mut self) {
        self.close("incoming IRC line exceeds 8703 bytes".to_owned());
    }

    fn sync_monitored_state(&mut self) {
        self.state
            .sync_monitored(&self.monitor, self.isupport.value());
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
