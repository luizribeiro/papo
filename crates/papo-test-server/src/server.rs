use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{certificate, directory::TemporaryDirectory, ergo::Configuration};

const START_ATTEMPTS: usize = 5;

/// A child Ergo process with isolated storage and loopback listeners.
pub struct TestServer {
    child: Child,
    directory: TemporaryDirectory,
    configuration: Configuration,
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
        Self::start_persistent()
    }

    /// Starts Ergo with its shipped feature and history defaults.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start_default() -> io::Result<Self> {
        Self::start_with(Configuration::new)
    }

    /// Starts Ergo with always-on accounts and persistent history.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start_persistent() -> io::Result<Self> {
        Self::start_with(Configuration::persistent)
    }

    /// Starts Ergo with STS advertising its isolated TLS listener.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start_with_sts() -> io::Result<Self> {
        Self::start_with(|directory| {
            let mut configuration = Configuration::persistent(directory)?;
            configuration.enable_sts();
            Ok(configuration)
        })
    }

    /// Starts Ergo with ephemeral history for unregistered channels.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start_with_ephemeral_channels() -> io::Result<Self> {
        Self::start_with(|directory| {
            let mut configuration = Configuration::new(directory)?;
            configuration.disable_persistent_unregistered_channels();
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

    /// Creates a controllable forwarding endpoint for one plaintext client.
    ///
    /// # Errors
    ///
    /// Returns an error if its loopback listener cannot be created.
    pub fn fault_proxy(&self) -> io::Result<crate::FaultProxy> {
        crate::FaultProxy::start(self.plaintext)
    }

    /// Changes an account password while preserving the running server.
    ///
    /// # Errors
    ///
    /// Returns an error if authentication or the `NickServ` command fails.
    pub fn change_account_password(
        &self,
        account: &str,
        current: &str,
        replacement: &str,
    ) -> io::Result<()> {
        crate::change_account_password_at(self.plaintext, account, current, replacement)
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
        self.child = self
            .configuration
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
                configuration: config,
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
        let addresses = [
            self.plaintext,
            self.tls,
            self.websocket,
            self.secure_websocket,
        ];
        wait_until_ready_until("Ergo", addresses, deadline, || self.child.try_wait())
    }
}

pub(crate) fn wait_until_ready_until<const N: usize>(
    server_name: &str,
    addresses: [SocketAddr; N],
    deadline: Instant,
    mut child_status: impl FnMut() -> io::Result<Option<ExitStatus>>,
) -> io::Result<()> {
    while Instant::now() < deadline {
        if let Some(status) = child_status()? {
            return Err(io::Error::other(format!(
                "{server_name} exited with {status}"
            )));
        }
        if addresses
            .iter()
            .all(|address| TcpStream::connect(address).is_ok())
            && answers_irc(addresses[0])
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{server_name} did not start"),
    ))
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
        io::{self, Read, Write},
        net::{TcpListener, TcpStream},
        path::Path,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use super::{Configuration, TestServer, answers_irc, wait_until_ready_until};

    #[test]
    fn irc_probe_requires_a_reply_from_an_open_port() {
        let silent_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let silent_address = silent_listener.local_addr().unwrap();
        let (release_sender, release_receiver) = mpsc::channel();
        thread::scope(|scope| {
            scope.spawn(move || {
                let (mut connection, _) = silent_listener.accept().unwrap();
                let mut ping = [0; 18];
                connection.read_exact(&mut ping).unwrap();
                assert_eq!(&ping, b"PING :papo-ready\r\n");
                release_receiver.recv().unwrap();
            });
            let answered = answers_irc(silent_address);
            let mut unblock = TcpStream::connect(silent_address).unwrap();
            unblock.write_all(b"PING :papo-ready\r\n").unwrap();
            release_sender.send(()).unwrap();
            assert!(!answered);
        });

        let answering_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let answering_address = answering_listener.local_addr().unwrap();
        thread::scope(|scope| {
            scope.spawn(move || {
                let (mut connection, _) = answering_listener.accept().unwrap();
                let mut ping = [0; 18];
                connection.read_exact(&mut ping).unwrap();
                assert_eq!(&ping, b"PING :papo-ready\r\n");
                connection
                    .write_all(b":irc.example PONG irc.example :papo-ready\r\n")
                    .unwrap();
            });
            assert!(answers_irc(answering_address));
        });

        let closed_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let closed_address = closed_listener.local_addr().unwrap();
        drop(closed_listener);
        assert!(!answers_irc(closed_address));
    }

    #[test]
    fn readiness_times_out_when_every_port_accepts_but_irc_is_silent() {
        let listeners =
            std::array::from_fn::<_, 4, _>(|_| TcpListener::bind(("127.0.0.1", 0)).unwrap());
        let addresses = listeners
            .each_ref()
            .map(|listener| listener.local_addr().unwrap());
        let error = wait_until_ready_until(
            "Ergo",
            addresses,
            Instant::now() + Duration::from_millis(20),
            || Ok::<_, io::Error>(None),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "Ergo did not start");
    }

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
