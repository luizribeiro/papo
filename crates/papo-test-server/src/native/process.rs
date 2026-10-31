use std::{
    fmt,
    io::{self, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
};

const STDERR_LIMIT: usize = 4096;
const START_ATTEMPTS: usize = 3;
pub(crate) struct ServerChild {
    child: Child,
    stderr: Arc<Mutex<Tail>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl ServerChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut pipe = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("server stderr pipe is unavailable"))?;
        let stderr = Arc::new(Mutex::new(Tail::default()));
        let captured = Arc::clone(&stderr);
        let reader = thread::spawn(move || {
            let mut chunk = [0; 1024];
            while let Ok(count) = pipe.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                if let Ok(mut tail) = captured.lock() {
                    tail.extend(&chunk[..count]);
                }
            }
        });
        Ok(Self {
            child,
            stderr,
            reader: Some(reader),
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn try_wait(&mut self, name: &str) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.child.try_wait()? {
            self.finish_reader();
            return Err(self.failure(io::ErrorKind::Other, format!("{name} exited with {status}")));
        }
        Ok(None)
    }

    pub(crate) fn stop(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
            self.child.wait()?;
        }
        self.finish_reader();
        Ok(())
    }

    fn finish_reader(&mut self) {
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    fn failure(&self, kind: io::ErrorKind, summary: String) -> io::Error {
        let stderr = self
            .stderr
            .lock()
            .map_or_else(|_| String::new(), |tail| tail.text());
        io::Error::new(kind, Failure::new(summary, stderr))
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) fn retry_start<T>(mut start: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for attempt in 1..=START_ATTEMPTS {
        match start() {
            Err(error) if attempt < START_ATTEMPTS && retryable(&error) => {}
            result => return result,
        }
    }
    unreachable!()
}

fn retryable(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AddrInUse
        || error
            .get_ref()
            .and_then(|source| source.downcast_ref::<Failure>())
            .is_some_and(|failure| failure.retryable)
}

struct Failure {
    summary: String,
    stderr: String,
    retryable: bool,
}

impl Failure {
    fn new(summary: String, stderr: String) -> Self {
        let diagnostic = stderr.to_ascii_lowercase();
        let retryable = ["address already in use", "address in use", "eaddrinuse"]
            .iter()
            .any(|phrase| diagnostic.contains(phrase));
        Self {
            summary,
            stderr,
            retryable,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.summary)?;
        if !self.stderr.is_empty() {
            write!(formatter, ": {}", self.stderr.trim())?;
        }
        Ok(())
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for Failure {}

#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn extend(&mut self, input: &[u8]) {
        self.0.extend(input);
        let excess = self.0.len().saturating_sub(STDERR_LIMIT);
        self.0.drain(..excess);
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{Failure, STDERR_LIMIT, Tail, retry_start};

    #[test]
    fn retries_an_address_in_use_startup_exit() {
        let mut attempts = 0;
        let port = retry_start(|| {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::other(Failure::new(
                    "soju exited with exit status: 1".into(),
                    "listen tcp 127.0.0.1:43821: bind: address already in use".into(),
                )))
            } else {
                Ok(43822)
            }
        })
        .unwrap();

        assert_eq!((port, attempts), (43822, 2));
    }

    #[test]
    fn does_not_retry_a_configuration_error() {
        let mut attempts = 0;
        let error = retry_start::<()>(|| {
            attempts += 1;
            Err(io::Error::other(Failure::new(
                "InspIRCd exited with exit status: 1".into(),
                "unknown configuration directive: link".into(),
            )))
        })
        .unwrap_err();

        assert_eq!(attempts, 1);
        assert!(
            error
                .to_string()
                .contains("unknown configuration directive")
        );
    }

    #[test]
    fn bounds_stderr_that_keeps_arriving() {
        let mut tail = Tail::default();
        for ordinal in 0..10_000_u32 {
            tail.extend(format!("diagnostic-{ordinal:05}\n").as_bytes());
        }
        assert_eq!(tail.0.len(), STDERR_LIMIT);
        let output = tail.text();
        assert!(output.ends_with("diagnostic-09999\n") && !output.contains("diagnostic-00000"));
    }
}
