use std::{env, io, net::SocketAddr, path::PathBuf};

#[cfg(not(target_family = "wasm"))]
use crate::TestServer;

/// Connection details for a running test server.
pub struct Server {
    plaintext: SocketAddr,
    tls: SocketAddr,
    ca: PathBuf,
    #[cfg(not(target_family = "wasm"))]
    _owner: Option<TestServer>,
}

impl Server {
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

    /// Returns the CA certificate path visible to the current process.
    #[must_use]
    pub fn ca_pem_path(&self) -> &std::path::Path {
        &self.ca
    }
}

/// Uses the server named by `PAPO_TEST_*`, or starts a local one on native targets.
///
/// # Errors
///
/// Returns an error for incomplete or invalid environment values, when a local
/// server cannot be started, or when no server is provided on a wasm target.
pub fn from_env_or_start() -> io::Result<Server> {
    let values = [
        env::var("PAPO_TEST_PLAINTEXT").ok(),
        env::var("PAPO_TEST_TLS").ok(),
        env::var("PAPO_TEST_CA").ok(),
    ];
    if let Some(server) = from_values(values)? {
        return Ok(server);
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let owner = TestServer::start()?;
        Ok(Server {
            plaintext: owner.plaintext_addr(),
            tls: owner.tls_addr(),
            ca: owner.ca_pem_path(),
            _owner: Some(owner),
        })
    }
    #[cfg(target_family = "wasm")]
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "PAPO_TEST_PLAINTEXT, PAPO_TEST_TLS, and PAPO_TEST_CA are unset",
    ))
}

fn from_values(values: [Option<String>; 3]) -> io::Result<Option<Server>> {
    let [plaintext, tls, ca] = values;
    match (plaintext, tls, ca) {
        (None, None, None) => Ok(None),
        (Some(plaintext), Some(tls), Some(ca)) => Ok(Some(Server {
            plaintext: parse_address("PAPO_TEST_PLAINTEXT", &plaintext)?,
            tls: parse_address("PAPO_TEST_TLS", &tls)?,
            ca: ca.into(),
            #[cfg(not(target_family = "wasm"))]
            _owner: None,
        })),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PAPO_TEST_PLAINTEXT, PAPO_TEST_TLS, and PAPO_TEST_CA must be set together",
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
    use super::from_values;

    #[test]
    fn accepts_complete_environment_values() {
        let server = from_values([
            Some("127.0.0.1:6667".into()),
            Some("127.0.0.1:6697".into()),
            Some("target/papo-test-ca.pem".into()),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(server.plaintext_addr().port(), 6667);
        assert_eq!(server.tls_addr().port(), 6697);
        assert_eq!(
            server.ca_pem_path(),
            std::path::Path::new("target/papo-test-ca.pem")
        );
    }

    #[test]
    fn rejects_partial_or_invalid_environment_values() {
        assert!(from_values([Some("127.0.0.1:6667".into()), None, None]).is_err());
        assert!(
            from_values([
                Some("localhost:6667".into()),
                Some("127.0.0.1:6697".into()),
                Some("ca.pem".into())
            ])
            .is_err()
        );
    }
}
