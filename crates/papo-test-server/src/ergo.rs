use std::{env, io, process::Command};

use serde_json::json;

use crate::directory::TemporaryDirectory;

const WEBIRC_PASSWORD_HASH: &str = "$2a$04$04hAlnfNNCBj4DZF1nKiRuX6Z.HVpSDJXEFV8gW9E/CbJ04v0M1OO";
const OPER_PASSWORD_HASH: &str = "$2a$04$aSFvaLz6O6WDI7s6wA/YWugI4u1mOn47C6dxsW.TuuHhXrCfcVy/i";

#[derive(Clone)]
pub(crate) struct Configuration {
    pub(crate) plaintext: std::net::SocketAddr,
    pub(crate) tls: std::net::SocketAddr,
    pub(crate) websocket: std::net::SocketAddr,
    pub(crate) secure_websocket: std::net::SocketAddr,
    pub(crate) overrides: Vec<(&'static str, String)>,
}

impl Configuration {
    pub(crate) fn new(directory: &TemporaryDirectory) -> io::Result<Self> {
        let [plaintext, tls, websocket, secure_websocket] = crate::ports::allocate_loopback()?;
        Ok(Self::from_addresses(
            directory,
            [plaintext, tls, websocket, secure_websocket],
        ))
    }

    pub(crate) fn persistent(directory: &TemporaryDirectory) -> io::Result<Self> {
        let mut configuration = Self::new(directory)?;
        configuration.enable_persistence();
        Ok(configuration)
    }

    #[cfg(test)]
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

    pub(crate) fn disable_persistent_unregistered_channels(&mut self) {
        self.set_override("ERGO__HISTORY__PERSISTENT__UNREGISTERED_CHANNELS", "false");
    }

    fn enable_persistence(&mut self) {
        self.set_override("ERGO__ACCOUNTS__MULTICLIENT__ALWAYS_ON", "mandatory");
        self.set_override("ERGO__HISTORY__PERSISTENT__ENABLED", "true");
        self.set_override("ERGO__HISTORY__PERSISTENT__UNREGISTERED_CHANNELS", "true");
    }

    fn set_override(&mut self, key: &'static str, value: &str) {
        if let Some((_, configured)) = self
            .overrides
            .iter_mut()
            .find(|(configured, _)| *configured == key)
        {
            *configured = value.into();
        } else {
            self.overrides.push((key, value.into()));
        }
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
            (
                "ERGO__SERVER__WEBIRC",
                json!([{
                    "password": WEBIRC_PASSWORD_HASH,
                    "hosts": ["127.0.0.1"],
                    "accept-hostname": true
                }])
                .to_string(),
            ),
            (
                "ERGO__HISTORY__RETENTION__ALLOW_INDIVIDUAL_DELETE",
                "true".into(),
            ),
            (
                "ERGO__OPERS",
                json!({
                    crate::OPER_NAME: {
                        "class": "server-admin",
                        "hidden": false,
                        "password": OPER_PASSWORD_HASH
                    }
                })
                .to_string(),
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
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use crate::directory::TemporaryDirectory;

    use super::{Configuration, OPER_PASSWORD_HASH};

    fn configuration(directory: &TemporaryDirectory) -> Configuration {
        Configuration::at_addresses(
            directory,
            [7667, 7668, 7669, 7670]
                .map(|port| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)),
        )
    }

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
        assert!(config.overrides.iter().any(|(key, value)| {
            *key == "ERGO__HISTORY__RETENTION__ALLOW_INDIVIDUAL_DELETE" && value == "true"
        }));
        assert!(
            config
                .overrides
                .iter()
                .any(|(key, value)| *key == "ERGO__OPERS"
                    && value.contains(crate::OPER_NAME)
                    && value.contains(OPER_PASSWORD_HASH))
        );
    }

    #[test]
    fn enables_sts_with_the_tls_listener_and_one_hour_duration() {
        let directory = TemporaryDirectory::create().unwrap();
        let mut config = configuration(&directory);
        config.enable_sts();

        assert_eq!(
            &config.overrides[8..],
            &[
                ("ERGO__SERVER__STS__ENABLED", "true".into()),
                ("ERGO__SERVER__STS__PORT", "7668".into()),
                ("ERGO__SERVER__STS__DURATION", "1h".into()),
            ]
        );
    }

    #[test]
    fn disables_persistent_history_for_unregistered_channels() {
        let directory = TemporaryDirectory::create().unwrap();
        let mut config = configuration(&directory);
        config.disable_persistent_unregistered_channels();

        let override_value = config
            .overrides
            .iter()
            .find(|(key, _)| *key == "ERGO__HISTORY__PERSISTENT__UNREGISTERED_CHANNELS");
        assert_eq!(
            override_value,
            Some(&(
                "ERGO__HISTORY__PERSISTENT__UNREGISTERED_CHANNELS",
                "false".into()
            ))
        );
    }

    #[test]
    fn persistent_profile_enables_always_on_accounts_and_all_history() {
        let directory = TemporaryDirectory::create().unwrap();
        let config = Configuration::persistent(&directory).unwrap();

        for expected in [
            ("ERGO__ACCOUNTS__MULTICLIENT__ALWAYS_ON", "mandatory"),
            ("ERGO__HISTORY__PERSISTENT__ENABLED", "true"),
            ("ERGO__HISTORY__PERSISTENT__UNREGISTERED_CHANNELS", "true"),
        ] {
            assert!(
                config
                    .overrides
                    .iter()
                    .any(|(key, value)| (*key, value.as_str()) == expected)
            );
        }
    }
}
