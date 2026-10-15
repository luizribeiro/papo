//! Sans-I/O IRC session state and protocol behavior.
//! Time and randomness enter through explicit inputs.

mod caps;
mod framing;
mod isupport;
mod keepalive;
mod registration;
mod sasl;
pub mod sts;

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use papo_proto::{BuildError, MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message, MessageRef};

use caps::Caps;
use framing::Framer;
use isupport::IsupportState;
use keepalive::Keepalive;
use registration::Registration;
pub use sasl::{Entropy, Sasl};

const MAX_INCOMING_LINE_LEN: usize = MAX_TAG_SECTION_LEN + MAX_MESSAGE_LEN;
const DEFAULT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(60);

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

/// Configuration for an IRC session.
#[derive(Clone, Debug, Eq, PartialEq)]
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
            wanted_caps: DEFAULT_WANTED_CAPS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            alt_nicks: Vec::new(),
            sasl: None,
            require_sasl: false,
            device: None,
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
}

/// Sans-I/O state for one IRC client session.
pub struct Session {
    config: Config,
    caps: Caps,
    framer: Framer,
    isupport: IsupportState,
    keepalive: Keepalive,
    registration: Registration,
    entropy: Option<Box<dyn Entropy + Send>>,
    outbox: Outbox,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("config", &self.config)
            .field("caps", &self.caps)
            .field("framer", &self.framer)
            .field("isupport", &self.isupport)
            .field("keepalive", &self.keepalive)
            .field("registration", &self.registration)
            .field("has_entropy", &self.entropy.is_some())
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
    events: VecDeque<Event>,
    close_requested: bool,
}

impl Outbox {
    fn transmit_raw(&mut self, line: Vec<u8>) {
        self.transmits.push_back(line);
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

    fn reset_connection(&mut self) {
        self.transmits.clear();
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
        let registration = Registration::new(&config);
        let isupport = IsupportState::new(config.decoding);
        Self {
            config,
            caps: Caps::default(),
            framer: Framer::default(),
            isupport,
            keepalive: Keepalive,
            registration,
            entropy,
            outbox: Outbox::default(),
        }
    }

    /// Marks the transport as connected at `now`.
    pub fn handle_connected(&mut self, now: Instant) {
        self.framer.reset();
        self.outbox.reset_connection();
        self.caps.handle_connected();
        self.isupport.handle_connected();
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
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            return;
        }
        if line.len() > MAX_INCOMING_LINE_LEN {
            self.close_for_long_line();
            return;
        }

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
        self.registration.handle_timeout(
            now,
            self.caps.enabled(),
            self.isupport.value(),
            &mut self.outbox,
        );
        self.keepalive.handle_timeout(now, &mut self.outbox);
        self.apply_close_request();
    }

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.framer.close();
        self.outbox.reset_connection();
        self.registration.handle_disconnected();
    }

    /// Validates and queues a user-originated raw message for transmission.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if `message` cannot be represented as a valid IRC wire line.
    pub fn send(&mut self, _now: Instant, message: Message) -> Result<(), BuildError> {
        let line = message.to_line()?;
        drop(message);
        self.outbox.transmit_raw(line.into());
        Ok(())
    }

    /// Returns the next CRLF-terminated wire line to transmit.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
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
            self.keepalive.next_deadline(),
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

    /// Reports whether connection registration has completed.
    #[must_use]
    pub const fn is_registered(&self) -> bool {
        self.registration.is_registered()
    }

    fn malformed(&mut self, line: &[u8]) {
        self.outbox.event(Event::Malformed {
            line: line.to_vec(),
        });
    }

    fn dispatch_message(&mut self, now: Instant, message: &MessageRef<'_>) {
        let handled = self.registration.handle_message(
            now,
            message,
            &self.config,
            self.caps.enabled(),
            self.isupport.value(),
            &mut self.outbox,
        ) == Handled::Yes
            || self.caps.handle_message(
                message,
                &self.config,
                self.registration.authentication_finished(),
                &mut self.outbox,
            ) == Handled::Yes
            || self.isupport.handle_message(message) == Handled::Yes
            || self
                .keepalive
                .handle_message(now, message, &mut self.outbox)
                == Handled::Yes;
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
        }
    }
}
