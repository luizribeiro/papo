use std::{env, io, net::SocketAddr, path::PathBuf, process::Command};

use super::{
    atheme, certificate, directory::TemporaryDirectory, process, process::ServerChild,
    solanum_config::Configuration,
};

/// A Solanum process linked to Atheme for the target runner.
pub struct TestServer {
    children: Vec<ServerChild>,
    directory: TemporaryDirectory,
    plaintext: SocketAddr,
    tls: SocketAddr,
}

impl TestServer {
    /// Starts Solanum and Atheme and waits for their services link.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration, process startup, or readiness fails.
    pub fn start() -> io::Result<Self> {
        process::retry_start(|| Self::start_once(false))
    }
    /// Starts the fixture with a required connection password.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration, process startup, or readiness fails.
    pub fn start_with_password() -> io::Result<Self> {
        process::retry_start(|| Self::start_once(true))
    }

    fn start_once(require_password: bool) -> io::Result<Self> {
        let directory = TemporaryDirectory::create()?;
        certificate::generate(directory.path())?;
        let config = Configuration::new(&directory, require_password)?;
        let mut ircd = ServerChild::spawn(
            Command::new(
                env::var("SOLANUM_BIN").map_err(|_| io::Error::other("SOLANUM_BIN is unset"))?,
            )
            .args(["-foreground", "-configfile"])
            .arg(directory.path().join("solanum.conf"))
            .arg("-logfile")
            .arg(directory.path().join("solanum.log"))
            .arg("-pidfile")
            .arg(directory.path().join("solanum.pid"))
            .env("BANDB_DBPATH", directory.path().join("ban.db"))
            .current_dir(directory.path()),
        )?;
        super::server::wait_for_child("Solanum", [config.plaintext, config.tls], &mut ircd)?;
        let atheme = atheme::link(&directory, "Solanum", &mut ircd)?;
        Ok(Self {
            children: vec![ircd, atheme],
            directory,
            plaintext: config.plaintext,
            tls: config.tls,
        })
    }
    /// Returns the plaintext and TLS listeners.
    #[must_use]
    pub const fn endpoints(&self) -> [SocketAddr; 2] {
        [self.plaintext, self.tls]
    }
    /// Returns the generated CA certificate path.
    #[must_use]
    pub fn ca_pem_path(&self) -> PathBuf {
        self.directory.path().join("ca.pem")
    }

    /// Returns the Solanum process identifier.
    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.children[0].id()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        for child in self.children.iter_mut().rev() {
            let _ = child.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_client::lines_until;

    #[test]
    fn skips_the_ident_lookup() {
        let server = super::TestServer::start().unwrap();
        let lines = lines_until(server.plaintext, "papo-ident", b"", |line| {
            line.split(' ').nth(1) == Some("001")
        });
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with(":*** Ident disabled, not checking ident")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("Checking Ident")),
            "{lines:?}"
        );
    }

    #[test]
    fn registers_an_account_through_linked_atheme() {
        let server = super::TestServer::start().unwrap();
        let name = crate::unique("solanum-account");
        crate::account::register_atheme(server.plaintext, &name, "q7Hx-2mZp_9vLcT4wRfB").unwrap();
    }
}
