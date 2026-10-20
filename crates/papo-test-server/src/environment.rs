use std::{
    env, fmt, io,
    net::SocketAddr,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(not(target_family = "wasm"))]
use std::process::{Command, Stdio};

static NEXT_NAME: AtomicU64 = AtomicU64::new(0);
const RUNNER_HINT: &str = "run tests through the dev shell's test runner";

/// Connection details for the test server owned by the test runner.
#[derive(Clone, Eq, PartialEq)]
pub struct ServerInfo {
    plaintext: SocketAddr,
    tls: SocketAddr,
    websocket: String,
    secure_websocket: String,
    ca: PathBuf,
    bouncer: Option<BouncerInfo>,
}

/// Runner metadata for provisioning isolated networks on a local bouncer.
#[derive(Clone, Eq, PartialEq)]
pub struct BouncerInfo {
    username: String,
    password: String,
    config: PathBuf,
    upstream: SocketAddr,
}

/// Credentials bound to one bouncer network.
#[derive(Clone, Eq, PartialEq)]
pub struct BouncerLogin {
    username: String,
    password: String,
}

impl fmt::Debug for BouncerInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BouncerInfo")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("config", &self.config)
            .field("upstream", &self.upstream)
            .finish()
    }
}

impl fmt::Debug for ServerInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerInfo")
            .field("plaintext", &self.plaintext)
            .field("tls", &self.tls)
            .field("websocket", &self.websocket)
            .field("secure_websocket", &self.secure_websocket)
            .field("ca", &self.ca)
            .field("bouncer", &self.bouncer)
            .finish()
    }
}

impl fmt::Debug for BouncerLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BouncerLogin")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl ServerInfo {
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

    /// Returns the plaintext WebSocket URL.
    #[must_use]
    pub fn websocket_url(&self) -> &str {
        &self.websocket
    }

    /// Returns the TLS WebSocket URL.
    #[must_use]
    pub fn secure_websocket_url(&self) -> &str {
        &self.secure_websocket
    }

    /// Returns the CA certificate path visible to the current process.
    #[must_use]
    pub fn ca_pem_path(&self) -> &std::path::Path {
        &self.ca
    }

    /// Returns bouncer provisioning metadata when the selected profile uses one.
    #[must_use]
    pub const fn bouncer(&self) -> Option<&BouncerInfo> {
        self.bouncer.as_ref()
    }
}

impl BouncerInfo {
    /// Returns the bouncer account name.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Returns the bouncer account password.
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Returns the bouncer configuration path.
    #[must_use]
    pub fn config_path(&self) -> &std::path::Path {
        &self.config
    }

    /// Returns the upstream IRC listener address.
    #[must_use]
    pub const fn upstream_addr(&self) -> SocketAddr {
        self.upstream
    }

    /// Returns credentials for attaching another client to an existing network.
    ///
    /// # Errors
    ///
    /// Returns an error when `network` cannot be used as a managed network name.
    pub fn network_login(&self, network: &str) -> io::Result<BouncerLogin> {
        if network.is_empty() || network.contains([' ', '\r', '\n', '/', '\0']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid bouncer network name",
            ));
        }
        Ok(BouncerLogin {
            username: format!("{}/{network}", self.username),
            password: self.password.clone(),
        })
    }

    /// Creates a bouncer network whose upstream nickname is `nick`.
    ///
    /// # Errors
    ///
    /// Returns an error if the network name is invalid or soju rejects it.
    #[cfg(not(target_family = "wasm"))]
    pub fn provision_network(&self, nick: &str) -> io::Result<BouncerLogin> {
        let login = self.network_login(nick)?;
        let upstream = format!("irc+insecure://{}", self.upstream);
        let status = Command::new(
            env::var("SOJUCTL_BIN").map_err(|_| io::Error::other("SOJUCTL_BIN is unset"))?,
        )
        .args(["-config"])
        .arg(&self.config)
        .args([
            "user",
            "run",
            &self.username,
            "network",
            "create",
            "-addr",
            &upstream,
            "-name",
            nick,
            "-nick",
            nick,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "sojuctl could not provision network: {status}"
            )));
        }
        Ok(login)
    }
}

impl BouncerLogin {
    /// Returns the network-qualified bouncer username.
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Returns the bouncer password.
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }
}

/// Reads the test server endpoints supplied by the target runner.
///
/// # Panics
///
/// Panics when an endpoint is missing or malformed. Tests must run through the
/// development shell so its target runner can manage the fixture.
#[must_use]
pub fn from_env() -> ServerInfo {
    from_values(
        [
            env::var("PAPO_TEST_PLAINTEXT").ok(),
            env::var("PAPO_TEST_TLS").ok(),
            env::var("PAPO_TEST_WS").ok(),
            env::var("PAPO_TEST_WSS").ok(),
            env::var("PAPO_TEST_CA").ok(),
        ],
        [
            env::var("PAPO_TEST_BOUNCER_USER").ok(),
            env::var("PAPO_TEST_BOUNCER_PASSWORD").ok(),
            env::var("PAPO_TEST_BOUNCER_CONFIG").ok(),
            env::var("PAPO_TEST_UPSTREAM").ok(),
        ],
    )
    .unwrap_or_else(|error| panic!("{error}"))
}

/// Returns a process-unique IRC name no longer than 32 bytes.
#[must_use]
pub fn unique(prefix: &str) -> String {
    let counter = NEXT_NAME.fetch_add(1, Ordering::Relaxed);
    let suffix = format!("-{}-{counter}", test_process_id());
    let available = 32_usize.saturating_sub(suffix.len());
    let prefix: String = prefix
        .chars()
        .scan(0, |length, character| {
            *length += character.len_utf8();
            (*length <= available).then_some(character)
        })
        .collect();
    prefix + &suffix
}

fn test_process_id() -> String {
    if let Ok(id) = env::var("PAPO_TEST_PID") {
        return id;
    }
    #[cfg(not(target_family = "wasm"))]
    {
        std::process::id().to_string()
    }
    #[cfg(target_family = "wasm")]
    {
        panic!("PAPO_TEST_PID is unset; {RUNNER_HINT}");
    }
}

fn from_values(
    values: [Option<String>; 5],
    bouncer: [Option<String>; 4],
) -> io::Result<ServerInfo> {
    if values.iter().all(Option::is_none) {
        return Err(io::Error::new(io::ErrorKind::NotFound, RUNNER_HINT));
    }
    let [plaintext, tls, websocket, secure_websocket, ca] = values;
    match (plaintext, tls, websocket, secure_websocket, ca) {
        (Some(plaintext), Some(tls), Some(websocket), Some(secure_websocket), Some(ca)) => {
            let bouncer = match bouncer {
                [None, None, None, None] => None,
                [Some(username), Some(password), Some(config), Some(upstream)] => {
                    Some(BouncerInfo {
                        username,
                        password,
                        config: config.into(),
                        upstream: parse_address("PAPO_TEST_UPSTREAM", &upstream)?,
                    })
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "PAPO test bouncer variables must be set together",
                    ));
                }
            };
            Ok(ServerInfo {
                plaintext: parse_address("PAPO_TEST_PLAINTEXT", &plaintext)?,
                tls: parse_address("PAPO_TEST_TLS", &tls)?,
                websocket,
                secure_websocket,
                ca: ca.into(),
                bouncer,
            })
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "PAPO_TEST_PLAINTEXT, PAPO_TEST_TLS, PAPO_TEST_WS, PAPO_TEST_WSS, and PAPO_TEST_CA must be set together; {RUNNER_HINT}"
            ),
        )),
    }
}

fn parse_address(name: &str, value: &str) -> io::Result<SocketAddr> {
    value
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, format!("{name}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{BouncerLogin, RUNNER_HINT, from_values, test_process_id, unique};

    #[test]
    fn distinguishes_unset_complete_and_partial_environments() {
        assert_eq!(
            from_values([None, None, None, None, None], [None, None, None, None])
                .unwrap_err()
                .to_string(),
            RUNNER_HINT
        );
        let server = from_values(
            [
                Some("127.0.0.1:6667".into()),
                Some("127.0.0.1:6697".into()),
                Some("ws://127.0.0.1:8097/".into()),
                Some("wss://localhost:8098/".into()),
                Some("target/papo-test-ca.pem".into()),
            ],
            [
                Some("papo-bouncer".into()),
                Some("unique-bouncer-password".into()),
                Some("target/soju.conf".into()),
                Some("127.0.0.1:7667".into()),
            ],
        )
        .unwrap();
        assert_eq!(server.plaintext_addr().port(), 6667);
        assert_eq!(server.tls_addr().port(), 6697);
        assert_eq!(server.websocket_url(), "ws://127.0.0.1:8097/");
        assert_eq!(server.secure_websocket_url(), "wss://localhost:8098/");
        assert_eq!(
            server.ca_pem_path(),
            std::path::Path::new("target/papo-test-ca.pem")
        );
        let bouncer = server.bouncer().unwrap();
        assert_eq!(bouncer.username(), "papo-bouncer");
        assert_eq!(bouncer.upstream_addr().port(), 7667);
        let existing = bouncer.network_login("reader").unwrap();
        assert_eq!(existing.username(), "papo-bouncer/reader");
        assert_eq!(existing.password(), "unique-bouncer-password");
        for invalid in [
            "",
            "two words",
            "network/name",
            "line\rbreak",
            "line\nbreak",
            "nul\0byte",
        ] {
            assert!(bouncer.network_login(invalid).is_err(), "{invalid:?}");
        }
        assert!(!format!("{server:?}").contains("unique-bouncer-password"));
        let login = BouncerLogin {
            username: "papo-bouncer/reader".into(),
            password: "another-unique-password".into(),
        };
        assert_eq!(login.username(), "papo-bouncer/reader");
        assert!(!format!("{login:?}").contains("another-unique-password"));
        for missing in 0..5 {
            let mut partial = [
                Some(String::new()),
                Some(String::new()),
                Some(String::new()),
                Some(String::new()),
                Some(String::new()),
            ];
            partial[missing] = None;
            assert!(
                from_values(partial, [None, None, None, None])
                    .unwrap_err()
                    .to_string()
                    .contains("set together")
            );
        }
    }

    #[test]
    fn unique_names_include_the_pid_and_counter_within_nicklen() {
        let first = unique("integration-registration-name-that-is-too-long");
        let second = unique("integration-registration-name-that-is-too-long");
        assert_ne!(first, second);
        assert!(first.contains(&format!("-{}-", test_process_id())));
        assert!(first.len() <= 32);
        assert!(second.len() <= 32);
    }
}
