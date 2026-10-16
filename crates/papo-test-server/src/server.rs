use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{certificate, directory::TemporaryDirectory, ergo::Configuration};

const START_ATTEMPTS: usize = 5;

/// A child Ergo process with isolated storage and loopback listeners.
pub struct TestServer {
    child: Child,
    directory: TemporaryDirectory,
    plaintext: SocketAddr,
    tls: SocketAddr,
    websocket: SocketAddr,
    secure_websocket: SocketAddr,
}

impl TestServer {
    /// Starts Ergo and waits until its plaintext and TLS listeners accept connections.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start() -> io::Result<Self> {
        Self::start_with(Configuration::new)
    }

    /// Starts Ergo with STS advertising its isolated TLS listener.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start_with_sts() -> io::Result<Self> {
        Self::start_with(|directory| {
            let mut configuration = Configuration::new(directory)?;
            configuration.enable_sts();
            Ok(configuration)
        })
    }

    /// Returns the plaintext IRC listener address.
    #[must_use]
    pub const fn plaintext_addr(&self) -> SocketAddr {
        self.plaintext
    }

    /// Returns the TLS IRC listener address.
    #[must_use]
    pub const fn tls_addr(&self) -> SocketAddr {
        self.tls
    }

    /// Returns the plaintext WebSocket listener address.
    #[must_use]
    pub const fn websocket_addr(&self) -> SocketAddr {
        self.websocket
    }

    /// Returns the TLS WebSocket listener address.
    #[must_use]
    pub const fn secure_websocket_addr(&self) -> SocketAddr {
        self.secure_websocket
    }

    /// Returns the generated CA certificate in PEM format.
    #[must_use]
    pub fn ca_pem_path(&self) -> PathBuf {
        self.directory.path().join("ca.pem")
    }

    /// Returns the Ergo child process identifier for lifecycle assertions.
    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    /// Stops Ergo while retaining its configuration and data directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the process status cannot be read or Ergo cannot be stopped.
    pub fn stop(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
            self.child.wait()?;
        }
        Ok(())
    }

    /// Restarts Ergo on the same listener addresses and waits until it is ready.
    ///
    /// # Errors
    ///
    /// Returns an error if Ergo cannot be stopped, spawned, or made ready.
    pub fn restart(&mut self) -> io::Result<()> {
        self.stop()?;
        let config = Configuration::at_addresses(
            &self.directory,
            [
                self.plaintext,
                self.tls,
                self.websocket,
                self.secure_websocket,
            ],
        );
        self.child = config
            .command("run", &self.directory)?
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        self.wait_until_ready()
    }

    fn start_with(
        mut configuration: impl FnMut(&TemporaryDirectory) -> io::Result<Configuration>,
    ) -> io::Result<Self> {
        let mut errors = Vec::with_capacity(START_ATTEMPTS);
        for _ in 0..START_ATTEMPTS {
            let directory = TemporaryDirectory::create()?;
            certificate::generate(directory.path())?;
            let config = configuration(&directory)?;
            let status = config
                .command("initdb", &directory)?
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "ergo initdb failed with {status}"
                )));
            }
            let child = config
                .command("run", &directory)?
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let mut server = Self {
                child,
                directory,
                plaintext: config.plaintext,
                tls: config.tls,
                websocket: config.websocket,
                secure_websocket: config.secure_websocket,
            };
            match server.wait_until_ready() {
                Ok(()) => return Ok(server),
                Err(error) => errors.push(error),
            }
        }
        let errors = errors
            .iter()
            .enumerate()
            .map(|(index, error)| format!("attempt {}: {error}", index + 1))
            .collect::<Vec<_>>()
            .join("; ");
        Err(io::Error::other(format!(
            "Ergo failed to become ready after {START_ATTEMPTS} attempts: {errors}"
        )))
    }

    fn wait_until_ready(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!("ergo exited with {status}")));
            }
            if [
                self.plaintext,
                self.tls,
                self.websocket,
                self.secure_websocket,
            ]
            .iter()
            .all(|address| TcpStream::connect(address).is_ok())
                && answers_irc(self.plaintext)
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Ergo did not start",
        ))
    }
}

/// A listening port alone can belong to another process, so readiness also requires an IRC reply.
fn answers_irc(address: SocketAddr) -> bool {
    let Ok(mut connection) = TcpStream::connect(address) else {
        return false;
    };
    if connection
        .set_read_timeout(Some(Duration::from_millis(500)))
        .is_err()
        || connection.write_all(b"PING :papo-ready\r\n").is_err()
    {
        return false;
    }
    let mut reply = [0; 512];
    matches!(connection.read(&mut reply), Ok(count) if count > 0)
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        path::Path,
        time::Duration,
    };

    use super::{Configuration, TestServer};

    #[test]
    fn retries_with_fresh_ports_after_a_bind_collision() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let occupied_address = occupied.local_addr().unwrap();
        let mut attempts = 0;
        let server = TestServer::start_with(|directory| {
            attempts += 1;
            let config = Configuration::new(directory)?;
            if attempts == 1 {
                Ok(Configuration::at_addresses(
                    directory,
                    [
                        occupied_address,
                        config.tls,
                        config.websocket,
                        config.secure_websocket,
                    ],
                ))
            } else {
                Ok(config)
            }
        })
        .unwrap();

        assert!((2..=5).contains(&attempts));
        assert_ne!(server.plaintext_addr(), occupied_address);
    }

    #[test]
    fn starts_greets_and_stops_ergo() {
        // This test owns a server to verify native process cleanup in isolation.
        let server = TestServer::start().unwrap();
        assert!(server.ca_pem_path().is_file());
        TcpStream::connect(server.tls_addr()).unwrap();
        let mut connection = TcpStream::connect(server.plaintext_addr()).unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        connection
            .write_all(b"NICK papo-fixture\r\nUSER papo 0 * :papo test fixture\r\n")
            .unwrap();
        let mut greeting = [0; 4096];
        let count = connection.read(&mut greeting).unwrap();
        assert!(String::from_utf8_lossy(&greeting[..count]).contains(" 001 papo-fixture "));
        let process = server.process_id();
        drop(server);
        assert!(!Path::new(&format!("/proc/{process}")).exists());
    }
}
