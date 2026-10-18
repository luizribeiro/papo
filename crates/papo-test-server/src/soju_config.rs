use std::{fs, io, net::SocketAddr, path::PathBuf};

use crate::directory::TemporaryDirectory;

pub(crate) struct Configuration {
    pub(crate) endpoints: [SocketAddr; 4],
    pub(crate) path: PathBuf,
}

impl Configuration {
    pub(crate) fn new(directory: &TemporaryDirectory) -> io::Result<Self> {
        Self::at_endpoints(directory, crate::ports::allocate_loopback()?)
    }

    fn at_endpoints(
        directory: &TemporaryDirectory,
        endpoints: [SocketAddr; 4],
    ) -> io::Result<Self> {
        let [plaintext, tls, websocket, secure_websocket] = endpoints;
        let path = directory.path().join("soju.conf");
        let body = format!(
            "listen irc+insecure://{plaintext}\n\
             listen ircs://{tls}\n\
             listen ws+insecure://{websocket}\n\
             listen wss://{secure_websocket}\n\
             listen unix+admin://{}\n\
             tls {} {}\n\
             hostname bouncer.papo.test\n\
             db sqlite3 {}\n\
             message-store db\n\
             http-origin *\n",
            directory.path().join("admin.sock").display(),
            directory.path().join("server.pem").display(),
            directory.path().join("server.key").display(),
            directory.path().join("soju.db").display(),
        );
        fs::write(&path, body)?;
        Ok(Self { endpoints, path })
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, net::SocketAddr};

    use crate::directory::TemporaryDirectory;

    use super::Configuration;

    #[test]
    fn writes_all_transports_admin_database_and_tls() {
        let directory = TemporaryDirectory::create().unwrap();
        let endpoints =
            [7667, 7668, 7669, 7670].map(|port| SocketAddr::from(([127, 0, 0, 1], port)));
        let config = Configuration::at_endpoints(&directory, endpoints).unwrap();
        let body = fs::read_to_string(config.path).unwrap();

        for line in [
            "listen irc+insecure://127.0.0.1:7667",
            "listen ircs://127.0.0.1:7668",
            "listen ws+insecure://127.0.0.1:7669",
            "listen wss://127.0.0.1:7670",
            "listen unix+admin://",
            "tls ",
            "db sqlite3 ",
            "message-store db",
            "http-origin *",
        ] {
            assert!(body.contains(line), "missing {line:?} in {body:?}");
        }
        assert_eq!(config.endpoints, endpoints);

        let allocated = Configuration::new(&directory).unwrap();
        assert_eq!(
            allocated
                .endpoints
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
    }
}
