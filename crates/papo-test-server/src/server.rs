use std::{
    io,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{certificate, directory::TemporaryDirectory, ergo::Configuration};

/// A child Ergo process with isolated storage and loopback listeners.
pub struct TestServer {
    child: Child,
    directory: TemporaryDirectory,
    plaintext: SocketAddr,
    tls: SocketAddr,
}

impl TestServer {
    /// Starts Ergo and waits until its plaintext and TLS listeners accept connections.
    ///
    /// # Errors
    ///
    /// Returns an error if setup fails or Ergo does not become ready in time.
    pub fn start() -> io::Result<Self> {
        let directory = TemporaryDirectory::create()?;
        certificate::generate(directory.path())?;
        let config = Configuration::new(&directory)?;
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
        };
        server.wait_until_ready()?;
        Ok(server)
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

    fn wait_until_ready(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!("ergo exited with {status}")));
            }
            if [self.plaintext, self.tls]
                .iter()
                .all(|address| TcpStream::connect(address).is_ok())
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
        net::TcpStream,
        path::Path,
        time::Duration,
    };

    use super::TestServer;

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
