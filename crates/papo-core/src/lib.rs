//! Sans-I/O IRC session state and protocol behavior.
//! Time and randomness enter through explicit inputs.

use std::{collections::VecDeque, time::Instant};

use papo_proto::Message;

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
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Config {
    /// The policy used to decode incoming lines.
    pub decoding: Decoding,
}

/// An event emitted by an IRC session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
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
#[derive(Debug)]
pub struct Session {
    transmits: VecDeque<Message>,
    events: VecDeque<Event>,
}

impl Session {
    /// Creates a session with no active connection.
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self {
            transmits: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    /// Marks the transport as connected at `now`.
    pub fn handle_connected(&mut self, _now: Instant) {}

    /// Supplies bytes read from a stream transport at `now`.
    pub fn handle_bytes(&mut self, _now: Instant, _bytes: &[u8]) {}

    /// Supplies one line from a transport that performs its own framing.
    pub fn handle_line(&mut self, _now: Instant, _line: &[u8]) {}

    /// Notifies the session that its current deadline was reached.
    pub fn handle_timeout(&mut self, _now: Instant) {}

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.transmits.clear();
    }

    /// Queues a user-originated raw message for transmission.
    pub fn send(&mut self, _now: Instant, message: Message) {
        self.transmits.push_back(message);
    }

    /// Returns the next CRLF-terminated wire line to transmit.
    ///
    /// # Panics
    ///
    /// Panics if a queued message cannot be represented as a valid IRC wire line.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.transmits.pop_front().map(|message| {
            message
                .to_line()
                .expect("message passed to Session::send is invalid")
                .into()
        })
    }

    /// Returns the next session event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Returns the next protocol deadline, if any.
    #[must_use]
    pub const fn poll_timeout(&self) -> Option<Instant> {
        None
    }
}
