use std::{
    env, io,
    net::SocketAddr,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

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
        process::retry_start(Self::start_once)
    }
    fn start_once() -> io::Result<Self> {
        let directory = TemporaryDirectory::create()?;
        certificate::generate(directory.path())?;
        let config = Configuration::new(&directory)?;
        let ircd = ServerChild::spawn(
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
        let mut server = Self {
            children: vec![ircd],
            directory,
            plaintext: config.plaintext,
            tls: config.tls,
        };
        server.wait_for(false)?;
        server
            .children
            .push(ServerChild::spawn(&mut atheme::command(
                &server.directory,
            )?)?);
        server.wait_for(true)?;
        Ok(server)
    }
    fn wait_for(&mut self, services: bool) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        if !services {
            let endpoints = [self.plaintext, self.tls];
            return super::server::wait_for_child_until(
                "Solanum",
                endpoints,
                deadline,
                &mut self.children[0],
            );
        }
        while Instant::now() < deadline {
            self.children[0].check("Solanum")?;
            self.children[1].check("Atheme")?;
            if atheme::linked(self.directory.path()) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(self.children[1].timeout("Atheme"))
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
    #[test]
    fn registers_an_account_through_linked_atheme() {
        let server = super::TestServer::start().unwrap();
        let name = crate::unique("solanum-account");
        crate::account::register_atheme(server.plaintext, &name, "q7Hx-2mZp_9vLcT4wRfB").unwrap();
    }
}
