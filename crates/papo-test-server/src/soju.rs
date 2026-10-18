use std::{
    env, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    TestServer as ErgoServer, certificate, directory::TemporaryDirectory,
    server::wait_until_ready_until, soju_config::Configuration,
};

pub(crate) const ACCOUNT: &str = "papo-bouncer";
pub(crate) const PASSWORD: &str = "mC8r-Zq4v_N7xLp2";

/// A process-owned soju bouncer connected to an isolated Ergo upstream.
pub struct TestServer {
    child: Child,
    ergo: ErgoServer,
    directory: TemporaryDirectory,
    configuration: PathBuf,
    endpoints: [SocketAddr; 4],
}

impl TestServer {
    /// Starts soju, creates its test account, and connects it to Ergo.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration, startup, or account setup fails.
    pub fn start() -> io::Result<Self> {
        let ergo = ErgoServer::start_default()?;
        let directory = TemporaryDirectory::create()?;
        certificate::generate(directory.path())?;
        let config = Configuration::new(&directory)?;
        let mut child = Command::new(required_env("SOJU_BIN")?)
            .args(["-config", path_text(&config.path)?])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        wait_until_ready_until(
            "soju",
            config.endpoints,
            Instant::now() + Duration::from_secs(10),
            || child.try_wait(),
        )?;
        let upstream = format!("irc+insecure://{}", ergo.plaintext_addr());
        run_control(
            &config.path,
            "create user",
            [
                "user",
                "create",
                "-username",
                ACCOUNT,
                "-password",
                PASSWORD,
                "-admin=true",
            ],
        )?;
        run_control(
            &config.path,
            "create network",
            [
                "user", "run", ACCOUNT, "network", "create", "-addr", &upstream, "-name", "ergo",
            ],
        )?;
        Ok(Self {
            child,
            ergo,
            directory,
            configuration: config.path,
            endpoints: config.endpoints,
        })
    }

    /// Returns plaintext, TLS, WebSocket, and secure WebSocket listeners.
    #[must_use]
    pub const fn endpoints(&self) -> [SocketAddr; 4] {
        self.endpoints
    }

    /// Returns the generated CA certificate path.
    #[must_use]
    pub fn ca_pem_path(&self) -> PathBuf {
        self.directory.path().join("ca.pem")
    }

    /// Returns the soju process identifier.
    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    /// Returns account, password, configuration, and upstream runner metadata.
    #[must_use]
    pub fn fixture_metadata(&self) -> (&str, &str, &Path, SocketAddr) {
        (
            ACCOUNT,
            PASSWORD,
            &self.configuration,
            self.ergo.plaintext_addr(),
        )
    }
}

fn run_control<'a>(
    config: &Path,
    operation: &str,
    command: impl IntoIterator<Item = &'a str>,
) -> io::Result<()> {
    let status = Command::new(required_env("SOJUCTL_BIN")?)
        .args(["-config", path_text(config)?])
        .args(command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "sojuctl could not {operation}: {status}"
        )))
    }
}

fn required_env(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| io::Error::other(format!("{name} is unset")))
}

fn path_text(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 temporary path"))
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
        io::{BufRead, BufReader, Write},
        net::TcpStream,
        time::Duration,
    };

    use super::{ACCOUNT, PASSWORD, TestServer};

    #[test]
    fn bouncer_account_reaches_the_ergo_upstream() {
        let server = TestServer::start().unwrap();
        let stream = TcpStream::connect(server.endpoints()[0]).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut connection = BufReader::new(stream);
        write!(
            connection.get_mut(),
            "PASS {PASSWORD}\r\nNICK {ACCOUNT}\r\nUSER {ACCOUNT}/ergo 0 * :papo bouncer test\r\n"
        )
        .unwrap();

        for _ in 0..128 {
            let mut line = String::new();
            assert_ne!(
                connection.read_line(&mut line).unwrap(),
                0,
                "soju closed before welcoming its configured account"
            );
            if line.contains(" 001 ") {
                assert!(line.contains(ACCOUNT), "{line:?}");
                return;
            }
        }
        panic!("soju did not welcome its configured account");
    }
}
