use std::{env, io, net::SocketAddr, path::PathBuf, process::Command};

use super::{
    atheme, certificate, directory::TemporaryDirectory, inspircd_config::Configuration, process,
    process::ServerChild,
};

/// An `InspIRCd` process linked to Atheme for the target runner.
pub struct TestServer {
    children: Vec<ServerChild>,
    directory: TemporaryDirectory,
    endpoints: [SocketAddr; 4],
}

impl TestServer {
    /// Starts `InspIRCd` and Atheme and waits for their services link.
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
                env::var("INSPIRCD_BIN").map_err(|_| io::Error::other("INSPIRCD_BIN is unset"))?,
            )
            .args(["--nofork", "--nopid", "--nolog", "--config"])
            .arg(directory.path().join("inspircd.conf"))
            .current_dir(directory.path()),
        )?;
        super::server::wait_for_child("InspIRCd", config.endpoints, &mut ircd)?;
        let atheme = atheme::link(&directory, "InspIRCd", &mut ircd)?;
        Ok(Self {
            children: vec![ircd, atheme],
            directory,
            endpoints: config.endpoints,
        })
    }

    /// Returns the plaintext, TLS, WebSocket, and secure WebSocket listeners.
    #[must_use]
    pub const fn endpoints(&self) -> [SocketAddr; 4] {
        self.endpoints
    }

    /// Returns the generated CA certificate path.
    #[must_use]
    pub fn ca_pem_path(&self) -> PathBuf {
        self.directory.path().join("ca.pem")
    }

    /// Returns the `InspIRCd` process identifier.
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
    #[test]
    fn registers_an_account_through_linked_atheme() {
        let server = super::TestServer::start().unwrap();
        let name = crate::unique("inspircd-account");
        crate::account::register_atheme(server.endpoints[0], &name, "q7Hx-2mZp_9vLcT4wRfB")
            .unwrap();
    }
}
