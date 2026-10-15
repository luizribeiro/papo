//! Shared support for transcript-driven protocol tests.

use std::time::{Duration, Instant};

use papo_core::{Config, Event, Session};
use papo_proto::Message;

/// A fluent driver for a [`Session`] with a manually advanced clock.
pub struct Transcript {
    session: Session,
    now: Instant,
    sent: Vec<String>,
}

impl Transcript {
    /// Creates a transcript at an arbitrary fixed starting instant.
    pub fn new(config: Config) -> Self {
        Self {
            session: Session::new(config),
            now: Instant::now(),
            sent: Vec::new(),
        }
    }

    /// Connects the session.
    pub fn connect(mut self) -> Self {
        self.session.handle_connected(self.now);
        self
    }

    /// Plays one server line without its line ending.
    pub fn server(mut self, line: &str) -> Self {
        self.session.handle_line(self.now, line.as_bytes());
        self
    }

    /// Queues a user-originated message.
    pub fn send(mut self, message: Message) -> Self {
        self.session.send(self.now, message);
        self
    }

    /// Asserts the next client line exactly, without its line ending.
    pub fn client(mut self, expected: &str) -> Self {
        let actual = self.session.poll_transmit().map(|line| {
            String::from_utf8(line)
                .expect("session transmits UTF-8 IRC messages")
                .trim_end_matches(['\r', '\n'])
                .to_owned()
        });
        if actual.as_deref() != Some(expected) {
            self.fail_client(expected, actual.as_deref());
        }
        self.sent.push(actual.expect("matching output is present"));
        self
    }

    /// Asserts that no client line is pending.
    pub fn no_client(mut self) -> Self {
        if let Some(actual) = self.session.poll_transmit() {
            let actual = String::from_utf8_lossy(&actual);
            self.fail_client("<nothing>", Some(actual.trim_end_matches(['\r', '\n'])));
        }
        self
    }

    /// Advances the manual clock and processes the resulting timeout.
    pub fn advance(mut self, duration: Duration) -> Self {
        self.now += duration;
        self.session.handle_timeout(self.now);
        self
    }

    /// Asserts that the next event satisfies `predicate`.
    pub fn event(mut self, predicate: impl FnOnce(&Event) -> bool) -> Self {
        let event = self.session.poll_event().unwrap_or_else(|| {
            panic!(
                "expected an event; client lines sent so far:\n{}",
                self.log()
            )
        });
        assert!(predicate(&event), "unexpected event: {event:?}");
        self
    }

    fn fail_client(&self, expected: &str, actual: Option<&str>) -> ! {
        panic!(
            "client output mismatch\nclient lines sent so far:\n{}\nexpected next:\n  C: {expected}\nactual next:\n  {}",
            self.log(),
            actual.map_or("<nothing>".to_owned(), |line| format!("C: {line}"))
        )
    }

    fn log(&self) -> String {
        if self.sent.is_empty() {
            "  <none>".to_owned()
        } else {
            self.sent
                .iter()
                .map(|line| format!("  C: {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}
