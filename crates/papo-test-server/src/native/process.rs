use std::{
    io::{self, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
};

const STDERR_LIMIT: usize = 4096;
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
        let stderr = stderr.trim();
        let message = if stderr.is_empty() {
            summary
        } else {
            format!("{summary}: {stderr}")
        };
        io::Error::new(kind, message)
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

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
    use super::{STDERR_LIMIT, Tail};

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
