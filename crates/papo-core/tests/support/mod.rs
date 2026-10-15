//! Shared support for transcript-driven protocol tests.
#![allow(dead_code)]

use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use papo_core::{Config, Event, Session};
use papo_proto::Message;

/// A fluent driver for a [`Session`] with a manually advanced clock.
pub struct Transcript {
    session: Session,
    now: Instant,
    sent: Vec<String>,
    ignore_rest: bool,
}

impl Transcript {
    /// Creates a transcript at an arbitrary fixed starting instant.
    pub fn new(config: Config) -> Self {
        Self {
            session: Session::new(config),
            now: Instant::now(),
            sent: Vec::new(),
            ignore_rest: false,
        }
    }

    /// Connects the session.
    pub fn connect(mut self) -> Self {
        self.session.handle_connected(self.now);
        self
    }

    /// Connects and asserts the default registration burst.
    pub fn connect_default(self) -> Self {
        self.connect()
            .client("CAP LS 302")
            .client("NICK papo")
            .client("USER papo 0 * papo")
    }

    /// Plays one server line without its line ending.
    pub fn server(self, line: &str) -> Self {
        self.server_bytes(line.as_bytes())
    }

    /// Plays one server line as raw bytes without its line ending.
    pub fn server_bytes(mut self, line: &[u8]) -> Self {
        self.session.handle_line(self.now, line);
        self
    }

    /// Queues a user-originated message.
    pub fn send(mut self, message: Message) -> Self {
        self.session
            .send(self.now, message)
            .expect("transcript user message is valid");
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

    /// Disconnects the session at the current transcript time.
    pub fn disconnect(mut self) -> Self {
        self.session.handle_disconnected(self.now);
        self
    }

    /// Inspects session state without changing it.
    pub fn inspect(self, inspect: impl FnOnce(&Session)) -> Self {
        inspect(&self.session);
        self
    }

    /// Asserts the next deadline relative to the current transcript time.
    pub fn deadline(self, after: Option<Duration>) -> Self {
        assert_eq!(
            self.session.poll_timeout(),
            after.map(|after| self.now + after)
        );
        self
    }

    /// Plays directives from a transcript trace file.
    pub fn load(mut self, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let trace = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        self = self.play_inner(&trace, &path.display().to_string());
        self
    }

    /// Plays directives from transcript trace contents.
    pub fn play(mut self, trace: &str) -> Self {
        self = self.play_inner(trace, "<inline trace>");
        self
    }

    fn play_inner(mut self, trace: &str, source: &str) -> Self {
        for (index, raw) in trace.lines().enumerate() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(server) = line.strip_prefix("S: ") {
                self = self.server(server);
            } else if let Some(client) = line.strip_prefix("C: ") {
                self = self.client(client);
            } else if let Some(seconds) = line
                .strip_prefix('+')
                .and_then(|line| line.strip_suffix('s'))
            {
                let seconds = seconds
                    .parse()
                    .unwrap_or_else(|_| panic!("{source}:{}: invalid duration: {line}", index + 1));
                self = self.advance(Duration::from_secs(seconds));
            } else {
                panic!("{source}:{}: invalid directive: {line}", index + 1);
            }
        }
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

    /// Asserts that no event is pending.
    pub fn no_event(mut self) -> Self {
        assert_eq!(self.session.poll_event(), None);
        self
    }

    /// Consumes events until one satisfies `predicate`.
    pub fn event_until(mut self, mut predicate: impl FnMut(&Event) -> bool) -> Self {
        loop {
            let event = self.session.poll_event().unwrap_or_else(|| {
                panic!(
                    "expected another event; client lines sent so far:\n{}",
                    self.log()
                )
            });
            if predicate(&event) {
                return self;
            }
        }
    }

    /// Discards any output that remains when this transcript ends.
    pub fn ignore_rest(mut self) {
        self.ignore_rest = true;
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

impl Drop for Transcript {
    fn drop(&mut self) {
        if self.ignore_rest || std::thread::panicking() {
            return;
        }
        let transmit = self.session.poll_transmit();
        let event = self.session.poll_event();
        if transmit.is_some() || event.is_some() {
            let transmit = transmit.as_deref().map_or("<none>".to_owned(), |line| {
                String::from_utf8_lossy(line).trim_end().to_owned()
            });
            panic!(
                "transcript ended with unexamined output\nnext client line: {transmit}\nnext event: {event:?}"
            );
        }
    }
}
