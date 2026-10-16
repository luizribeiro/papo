//! Runs a test server until its standard input closes.

#[cfg(not(target_family = "wasm"))]
use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
};

#[cfg(not(target_family = "wasm"))]
use papo_test_server::{TestServer, UnrealIrcdServer};

#[cfg(not(target_family = "wasm"))]
enum Server {
    Ergo(TestServer),
    UnrealIrcd(UnrealIrcdServer),
}

#[cfg(not(target_family = "wasm"))]
impl Server {
    fn start(kind: &str) -> io::Result<Self> {
        match kind {
            "ergo" => TestServer::start().map(Self::Ergo),
            "unrealircd" => UnrealIrcdServer::start().map(Self::UnrealIrcd),
            kind => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown PAPO_TEST_SERVER value {kind:?}"),
            )),
        }
    }

    fn endpoints(&self) -> (String, String, String, String) {
        match self {
            Self::Ergo(server) => (
                server.plaintext_addr().to_string(),
                server.tls_addr().to_string(),
                format!("ws://{}/", server.websocket_addr()),
                format!("wss://localhost:{}/", server.secure_websocket_addr().port()),
            ),
            Self::UnrealIrcd(server) => {
                let [plaintext, tls, websocket, secure_websocket] = server.endpoints();
                (
                    plaintext.to_string(),
                    tls.to_string(),
                    format!("ws://{websocket}/"),
                    format!("wss://localhost:{}/", secure_websocket.port()),
                )
            }
        }
    }

    fn ca_pem_path(&self) -> PathBuf {
        match self {
            Self::Ergo(server) => server.ca_pem_path(),
            Self::UnrealIrcd(server) => server.ca_pem_path(),
        }
    }

    fn process_id(&self) -> u32 {
        match self {
            Self::Ergo(server) => server.process_id(),
            Self::UnrealIrcd(server) => server.process_id(),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn main() -> io::Result<()> {
    // The runner helper owns the fixture outside every test process.
    let kind = env::var("PAPO_TEST_SERVER").unwrap_or_else(|_| "ergo".into());
    let server = Server::start(&kind)?;
    if let Some(path) = env::var_os("PAPO_TEST_ERGO_PID_FILE") {
        fs::write(path, server.process_id().to_string())?;
    }
    let (plaintext, tls, websocket, secure_websocket) = server.endpoints();
    println!("{plaintext}");
    println!("{tls}");
    println!("{websocket}");
    println!("{secure_websocket}");
    println!("{}", server.ca_pem_path().display());
    io::stdout().flush()?;
    io::copy(&mut io::stdin(), &mut io::sink())?;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn main() {}
