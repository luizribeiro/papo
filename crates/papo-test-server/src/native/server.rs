use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

use super::{
    certificate, directory::TemporaryDirectory, ergo::Configuration, ergo_ready::ReadySignal,
    process, process::ServerChild,
};

/// A child Ergo process with isolated storage and loopback listeners.
pub struct TestServer {
    child: ServerChild,
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

    /// Starts Ergo with STS and hostname lookup enabled for registration-order tests.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start_with_sts_and_hostname_lookup() -> io::Result<Self> {
        Self::start_with(|directory| {
            let mut configuration = Configuration::persistent(directory)?;
            configuration.enable_sts();
            configuration.enable_hostname_lookup();
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

    /// Creates a forwarding endpoint that presents Ergo's hostname notice before CAP LS.
    ///
    /// # Errors
    ///
    /// Returns an error if its loopback listener cannot be created.
    pub fn hostname_notice_proxy(&self) -> io::Result<crate::FaultProxy> {
        crate::FaultProxy::start_with_hostname_notice(self.plaintext)
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
        self.child.stop()
    }

    /// Restarts Ergo on the same listener addresses and waits until it is ready.
    ///
    /// # Errors
    ///
    /// Returns an error if Ergo cannot be stopped, spawned, or made ready.
    pub fn restart(&mut self) -> io::Result<()> {
        self.stop()?;
        self.child = spawn_ergo(&self.configuration, &self.directory)?;
        Ok(())
    }

    fn start_with(
        mut configuration: impl FnMut(&TemporaryDirectory) -> io::Result<Configuration>,
    ) -> io::Result<Self> {
        process::retry_start(|| {
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
            Ok(Self {
                child: spawn_ergo(&config, &directory)?,
                directory,
                plaintext: config.plaintext,
                tls: config.tls,
                websocket: config.websocket,
                secure_websocket: config.secure_websocket,
                configuration: config,
            })
        })
    }
}

fn spawn_ergo(
    configuration: &Configuration,
    directory: &TemporaryDirectory,
) -> io::Result<ServerChild> {
    let mut command = configuration.command("run", directory)?;
    let ready = ReadySignal::bind(directory.path(), &mut command)?;
    let mut child = ServerChild::spawn(&mut command)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    ready.wait(
        [
            configuration.plaintext,
            configuration.tls,
            configuration.websocket,
            configuration.secure_websocket,
        ],
        deadline,
        &mut child,
    )?;
    Ok(child)
}

pub(crate) fn wait_for_child_until<const N: usize>(
    server_name: &str,
    addresses: [SocketAddr; N],
    deadline: Instant,
    child: &mut ServerChild,
) -> io::Result<()> {
    let result = wait_until_ready_with(addresses, deadline, || {
        child
            .check(server_name)
            .map_err(|error| classify_bind_collision(error, &addresses))
    });
    if result
        .as_ref()
        .is_err_and(|error| error.kind() == io::ErrorKind::TimedOut)
    {
        Err(child.timeout(server_name))
    } else {
        result
    }
}

fn classify_bind_collision(error: io::Error, addresses: &[SocketAddr]) -> io::Error {
    if addresses
        .iter()
        .any(|address| TcpStream::connect(address).is_ok())
    {
        io::Error::new(io::ErrorKind::AddrInUse, error)
    } else {
        error
    }
}

fn wait_until_ready_with<const N: usize>(
    addresses: [SocketAddr; N],
    deadline: Instant,
    mut child_check: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    while Instant::now() < deadline {
        child_check()?;
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
        "server did not start",
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

    use super::{
        Configuration, TestServer, answers_irc, classify_bind_collision, wait_for_child_until,
        wait_until_ready_with,
    };
    use crate::native::process::ServerChild;

    #[test]
    fn only_an_occupied_listener_reclassifies_a_child_exit() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let occupied_address = occupied.local_addr().unwrap();
        let closed = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let closed_address = closed.local_addr().unwrap();
        drop(closed);

        let collision = classify_bind_collision(
            io::Error::other("Solanum exited with exit status: 1"),
            &[occupied_address],
        );
        let configuration = classify_bind_collision(
            io::Error::other("unknown configuration directive"),
            &[closed_address],
        );

        assert_eq!(collision.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(configuration.kind(), io::ErrorKind::Other);
        assert_eq!(configuration.to_string(), "unknown configuration directive");
    }

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
        let error = wait_until_ready_with(
            addresses,
            Instant::now() + Duration::from_millis(20),
            || Ok::<_, io::Error>(()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "server did not start");
    }

    #[test]
    fn readiness_timeout_uses_the_child_diagnostic() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut child =
            ServerChild::spawn(std::process::Command::new("sh").args(["-c", "sleep 60"])).unwrap();

        let error = wait_for_child_until(
            "InspIRCd",
            [address],
            Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
            &mut child,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "InspIRCd did not start");
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
                Ok(Configuration::from_addresses(
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
    fn rejects_readiness_from_another_ergo_process() {
        let occupied = TestServer::start().unwrap();
        let occupied_addresses = [
            occupied.plaintext_addr(),
            occupied.tls_addr(),
            occupied.websocket_addr(),
            occupied.secure_websocket_addr(),
        ];
        let mut attempts = 0;
        TestServer::start_with(|directory| {
            attempts += 1;
            if attempts == 1 {
                Ok(Configuration::from_addresses(directory, occupied_addresses))
            } else {
                Configuration::new(directory)
            }
        })
        .unwrap();

        assert_eq!(attempts, 2);
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
