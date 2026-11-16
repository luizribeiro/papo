//! A bounded, redacted record of every scenario client's wire lines.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{Arc, Mutex, PoisonError},
    time::Instant,
};

mod redact;
#[cfg(test)]
mod tests;

const LINES_PER_CLIENT: usize = 200;

#[derive(Clone)]
pub(crate) struct WireLog {
    started: Instant,
    clients: Arc<Mutex<Vec<ClientLog>>>,
}

struct ClientLog {
    name: String,
    lines: VecDeque<String>,
    dropped: usize,
}

#[derive(Clone, Copy)]
enum Direction {
    Sent,
    Received,
}

impl WireLog {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            clients: Arc::default(),
        }
    }

    /// Adds a client and returns the index its connections record under.
    pub(crate) fn add_client(&self) -> usize {
        let mut clients = self.clients();
        let name = format!("client {}", clients.len() + 1);
        clients.push(ClientLog {
            name,
            lines: VecDeque::new(),
            dropped: 0,
        });
        clients.len() - 1
    }

    pub(crate) fn rename(&self, client: usize, name: &str) {
        self.clients()[client].name = format!("client {} ({name})", client + 1);
    }

    pub(crate) fn render(&self) -> String {
        let mut output = String::from("wire log, in seconds since the scenario started:\n");
        for client in self.clients().iter() {
            _ = write!(output, "== {}", client.name);
            if client.dropped > 0 {
                _ = write!(output, " ({} earlier lines dropped)", client.dropped);
            }
            output.push('\n');
            for line in &client.lines {
                _ = writeln!(output, "  {line}");
            }
        }
        output
    }

    fn record(&self, client: usize, direction: Direction, line: &[u8]) {
        let arrow = match direction {
            Direction::Sent => "->",
            Direction::Received => "<-",
        };
        let line = format!(
            "{:.3} {arrow} {}",
            self.started.elapsed().as_secs_f64(),
            redact::redact(line)
        );
        let mut clients = self.clients();
        let client = &mut clients[client];
        client.lines.push_back(line);
        if client.lines.len() > LINES_PER_CLIENT {
            client.lines.pop_front();
            client.dropped += 1;
        }
    }

    fn clients(&self) -> std::sync::MutexGuard<'_, Vec<ClientLog>> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
