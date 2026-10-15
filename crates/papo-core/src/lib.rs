//! Sans-I/O IRC session state and protocol behavior.
//! Time and randomness enter through explicit inputs.

mod framing;
mod keepalive;

use std::{collections::VecDeque, time::Instant};

use papo_proto::{MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message, MessageRef};

use framing::Framer;
use keepalive::Keepalive;

const MAX_INCOMING_LINE_LEN: usize = MAX_TAG_SECTION_LEN + MAX_MESSAGE_LEN;

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
    config: Config,
    framer: Framer,
    keepalive: Keepalive,
    outbox: Outbox,
}

#[derive(Debug)]
enum Transmit {
    Message(Message),
    Raw(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Handled {
    Yes,
    No,
}

#[derive(Debug, Default)]
struct Outbox {
    transmits: VecDeque<Transmit>,
    events: VecDeque<Event>,
    close_requested: bool,
}

impl Outbox {
    fn transmit_raw(&mut self, line: Vec<u8>) {
        self.transmits.push_back(Transmit::Raw(line));
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
        Self {
            config,
            framer: Framer::default(),
            keepalive: Keepalive,
            outbox: Outbox::default(),
        }
    }

    /// Marks the transport as connected at `now`.
    pub fn handle_connected(&mut self, _now: Instant) {
        self.framer.reset();
        self.outbox.reset_connection();
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
            Err(_) if self.config.decoding == Decoding::Utf8OrLatin1 => {
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
        self.keepalive.handle_timeout(now, &mut self.outbox);
        self.apply_close_request();
    }

    /// Marks the transport as disconnected and drops per-connection state.
    pub fn handle_disconnected(&mut self, _now: Instant) {
        self.framer.reset();
        self.outbox.reset_connection();
    }

    /// Queues a user-originated raw message for transmission.
    pub fn send(&mut self, _now: Instant, message: Message) {
        self.outbox.transmits.push_back(Transmit::Message(message));
    }

    /// Returns the next CRLF-terminated wire line to transmit.
    ///
    /// # Panics
    ///
    /// Panics if a queued message cannot be represented as a valid IRC wire line.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.outbox
            .transmits
            .pop_front()
            .map(|transmit| match transmit {
                Transmit::Message(message) => message
                    .to_line()
                    .expect("message passed to Session::send is invalid")
                    .into(),
                Transmit::Raw(line) => line,
            })
    }

    /// Returns the next session event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.outbox.events.pop_front()
    }

    /// Returns the next protocol deadline, if any.
    #[must_use]
    pub fn poll_timeout(&self) -> Option<Instant> {
        [self.keepalive.next_deadline()].into_iter().flatten().min()
    }

    fn malformed(&mut self, line: &[u8]) {
        self.outbox.event(Event::Malformed {
            line: line.to_vec(),
        });
    }

    fn dispatch_message(&mut self, now: Instant, message: &MessageRef<'_>) {
        if self
            .keepalive
            .handle_message(now, message, &mut self.outbox)
            == Handled::No
        {
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
