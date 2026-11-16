//! A bounded, redacted record of every scenario client's wire lines.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll, ready},
    time::Instant,
};

use papo::__testing::wrapped::Stream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

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

    /// Wraps one connection of `client` so that every complete line it carries is recorded.
    pub(crate) fn tap(&self, client: usize, stream: Box<dyn Stream>) -> Box<dyn Stream> {
        Box::new(Tapped {
            inner: stream,
            log: self.clone(),
            client,
            partial: [Vec::new(), Vec::new()],
        })
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

struct Tapped {
    inner: Box<dyn Stream>,
    log: WireLog,
    client: usize,
    partial: [Vec<u8>; 2],
}

impl Tapped {
    fn observe(&mut self, direction: Direction, bytes: &[u8]) {
        let partial = &mut self.partial[direction as usize];
        for &byte in bytes {
            if byte == b'\n' {
                let line = std::mem::take(partial);
                let line = line.strip_suffix(b"\r").unwrap_or(&line);
                self.log.record(self.client, direction, line);
            } else {
                partial.push(byte);
            }
        }
    }
}

impl AsyncRead for Tapped {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(context, buffer))?;
        this.observe(Direction::Received, &buffer.filled()[before..]);
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Tapped {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let written = ready!(Pin::new(&mut this.inner).poll_write(context, buffer))?;
        this.observe(Direction::Sent, &buffer[..written]);
        Poll::Ready(Ok(written))
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}
