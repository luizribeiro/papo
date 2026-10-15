use std::{
    io::{self, Read, Write},
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
    api: SocketAddr,
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
            api: config.api,
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

    /// Creates a verified Ergo account through the local administrative API.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or Ergo refuses the account.
    pub fn register_account(&self, name: &str, password: &str) -> io::Result<()> {
        let body = serde_json::json!({"accountName": name, "passphrase": password}).to_string();
        let mut connection = TcpStream::connect(self.api)?;
        write!(
            connection,
            "POST /v1/ns/saregister HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer papo-test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.api,
            body.len(),
            body
        )?;
        let mut response = String::new();
        connection.read_to_string(&mut response)?;
        if response.starts_with("HTTP/1.1 200") && response.contains(r#""success":true"#) {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "Ergo refused account: {response}"
            )))
        }
    }

    fn wait_until_ready(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!("ergo exited with {status}")));
            }
            if [self.plaintext, self.tls, self.api]
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
        server
            .register_account("papo-account", "q7Hx-2mZp_9vLcT4wRfB")
            .unwrap();
        assert!(
            server
                .register_account("papo-account", "q7Hx-2mZp_9vLcT4wRfB")
                .is_err()
        );
        let process = server.child.id();
        drop(server);
        assert!(!Path::new(&format!("/proc/{process}")).exists());
    }
}
