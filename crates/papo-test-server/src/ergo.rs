use std::{env, io, net::TcpListener, process::Command};

use serde_json::json;

use crate::directory::TemporaryDirectory;

pub(crate) struct Configuration {
    pub(crate) plaintext: std::net::SocketAddr,
    pub(crate) tls: std::net::SocketAddr,
    pub(crate) api: std::net::SocketAddr,
    pub(crate) overrides: Vec<(&'static str, String)>,
}

impl Configuration {
    pub(crate) fn new(directory: &TemporaryDirectory) -> io::Result<Self> {
        let plaintext = TcpListener::bind(("127.0.0.1", 0))?;
        let tls = TcpListener::bind(("127.0.0.1", 0))?;
        let api = TcpListener::bind(("127.0.0.1", 0))?;
        let plaintext = plaintext.local_addr()?;
        let tls = tls.local_addr()?;
        let api = api.local_addr()?;
        let listeners = json!({
            plaintext.to_string(): {},
            tls.to_string(): {"tls": {
                "cert": directory.path().join("server.pem"),
                "key": directory.path().join("server.key")
            }}
        });
        let overrides = vec![
            ("ERGO__SERVER__LISTENERS", listeners.to_string()),
            (
                "ERGO__DATASTORE__PATH",
                directory.path().join("ircd.db").display().to_string(),
            ),
            ("ERGO__LANGUAGES__ENABLED", "false".into()),
            ("ERGO__ACCOUNTS__REGISTRATION__ENABLED", "true".into()),
            ("ERGO__API__ENABLED", "true".into()),
            ("ERGO__API__LISTENER", api.to_string()),
            (
                "ERGO__API__BEARER_TOKENS",
                json!(["papo-test-token"]).to_string(),
            ),
        ];
        Ok(Self {
            plaintext,
            tls,
            api,
            overrides,
        })
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
        assert_ne!(config.plaintext, config.api);
        assert_ne!(config.tls, config.api);
        assert!(config.plaintext.ip().is_loopback());
        assert!(config.tls.ip().is_loopback());
    }
}
