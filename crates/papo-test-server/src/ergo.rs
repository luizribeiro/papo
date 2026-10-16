use std::{env, io, net::TcpListener, process::Command};

use serde_json::json;

use crate::directory::TemporaryDirectory;

pub(crate) struct Configuration {
    pub(crate) plaintext: std::net::SocketAddr,
    pub(crate) tls: std::net::SocketAddr,
    pub(crate) websocket: std::net::SocketAddr,
    pub(crate) secure_websocket: std::net::SocketAddr,
    pub(crate) overrides: Vec<(&'static str, String)>,
}

impl Configuration {
    pub(crate) fn new(directory: &TemporaryDirectory) -> io::Result<Self> {
        let plaintext = TcpListener::bind(("127.0.0.1", 0))?;
        let tls = TcpListener::bind(("127.0.0.1", 0))?;
        let websocket = TcpListener::bind(("127.0.0.1", 0))?;
        let secure_websocket = TcpListener::bind(("127.0.0.1", 0))?;
        let plaintext = plaintext.local_addr()?;
        let tls = tls.local_addr()?;
        let websocket = websocket.local_addr()?;
        let secure_websocket = secure_websocket.local_addr()?;
        Ok(Self::from_addresses(
            directory,
            [plaintext, tls, websocket, secure_websocket],
        ))
    }

    pub(crate) fn at_addresses(
        directory: &TemporaryDirectory,
        addresses: [std::net::SocketAddr; 4],
    ) -> Self {
        Self::from_addresses(directory, addresses)
    }

    pub(crate) fn enable_sts(&mut self) {
        self.overrides
            .push(("ERGO__SERVER__STS__ENABLED", "true".into()));
        self.overrides
            .push(("ERGO__SERVER__STS__PORT", self.tls.port().to_string()));
        self.overrides
            .push(("ERGO__SERVER__STS__DURATION", "1h".into()));
    }

    fn from_addresses(
        directory: &TemporaryDirectory,
        addresses: [std::net::SocketAddr; 4],
    ) -> Self {
        let [plaintext, tls, websocket, secure_websocket] = addresses;
        let tls_config = || {
            json!({
                "cert": directory.path().join("server.pem"),
                "key": directory.path().join("server.key")
            })
        };
        let listeners = json!({
            plaintext.to_string(): {},
            tls.to_string(): {"tls": tls_config()},
            websocket.to_string(): {"websocket": true},
            secure_websocket.to_string(): {"websocket": true, "tls": tls_config()}
        });
        let overrides = vec![
            ("ERGO__SERVER__LISTENERS", listeners.to_string()),
            (
                "ERGO__DATASTORE__PATH",
                directory.path().join("ircd.db").display().to_string(),
            ),
            ("ERGO__DATASTORE__SQLITE__ENABLED", "true".into()),
            ("ERGO__LANGUAGES__ENABLED", "false".into()),
            ("ERGO__ACCOUNTS__REGISTRATION__ENABLED", "true".into()),
            ("ERGO__HISTORY__PERSISTENT__ENABLED", "true".into()),
            (
                "ERGO__HISTORY__PERSISTENT__UNREGISTERED_CHANNELS",
                "true".into(),
            ),
        ];
        Self {
            plaintext,
            tls,
            websocket,
            secure_websocket,
            overrides,
        }
    }

    pub(crate) fn command(
        &self,
        subcommand: &str,
        directory: &TemporaryDirectory,
    ) -> io::Result<Command> {
        let config = required_env("ERGO_CONFIG")?;
        let mut command = Command::new(required_env("ERGO_BIN")?);
        command
            .args([subcommand, "--conf", &config, "--quiet"])
            .current_dir(directory.path())
            .envs(self.overrides.iter().map(|(key, value)| (key, value)));
        Ok(command)
    }
}

fn required_env(name: &str) -> io::Result<String> {
    env::var(name).map_err(|_| io::Error::other(format!("{name} is unset")))
}

#[cfg(test)]
mod tests {
    use crate::directory::TemporaryDirectory;

    use super::Configuration;

    #[test]
    fn allocates_distinct_loopback_listeners() {
        let directory = TemporaryDirectory::create().unwrap();
        let config = Configuration::new(&directory).unwrap();
        assert_ne!(config.plaintext, config.tls);
        let addresses = [
            config.plaintext,
            config.tls,
            config.websocket,
            config.secure_websocket,
        ];
        assert_eq!(
            addresses
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
        assert!(addresses.iter().all(|address| address.ip().is_loopback()));
    }
}
