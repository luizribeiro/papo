//! Runs a test server until its standard input closes.

#[cfg(not(target_family = "wasm"))]
use std::{
    env, fs,
    io::{self, Write},
    path::PathBuf,
};

#[cfg(not(target_family = "wasm"))]
use papo_test_server::{
    DeploymentProfile, InspIrcdServer, SojuServer, SolanumServer, TestServer, UnrealIrcdServer,
    register_account_at,
};

#[cfg(not(target_family = "wasm"))]
enum Server {
    Ergo(TestServer),
    InspIrcd(InspIrcdServer),
    Solanum(SolanumServer),
    Soju(Box<SojuServer>),
    UnrealIrcd(UnrealIrcdServer),
}

#[cfg(not(target_family = "wasm"))]
impl Server {
    fn start(profile: DeploymentProfile) -> io::Result<Self> {
        match profile {
            DeploymentProfile::ErgoDefault => TestServer::start_default().map(Self::Ergo),
            DeploymentProfile::ErgoPersistent => TestServer::start_persistent().map(Self::Ergo),
            DeploymentProfile::InspIrcd => InspIrcdServer::start().map(Self::InspIrcd),
            DeploymentProfile::Solanum => SolanumServer::start().map(Self::Solanum),
            DeploymentProfile::UnrealIrcd => UnrealIrcdServer::start().map(Self::UnrealIrcd),
            DeploymentProfile::SojuErgo => SojuServer::start().map(Box::new).map(Self::Soju),
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
            Self::InspIrcd(server) => {
                let [plaintext, tls, websocket, secure_websocket] = server.endpoints();
                (
                    plaintext.to_string(),
                    tls.to_string(),
                    format!("ws://{websocket}/"),
                    format!("wss://localhost:{}/", secure_websocket.port()),
                )
            }
            Self::Solanum(server) => {
                let [plaintext, tls] = server.endpoints();
                let unavailable = String::new;
                (
                    plaintext.to_string(),
                    tls.to_string(),
                    unavailable(),
                    unavailable(),
                )
            }
            Self::Soju(server) => {
                let [plaintext, tls, websocket, secure_websocket] = server.endpoints();
                (
                    plaintext.to_string(),
                    tls.to_string(),
                    format!("ws://{websocket}/"),
                    format!("wss://localhost:{}/", secure_websocket.port()),
                )
            }
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
            Self::InspIrcd(server) => server.ca_pem_path(),
            Self::Solanum(server) => server.ca_pem_path(),
            Self::Soju(server) => server.ca_pem_path(),
            Self::UnrealIrcd(server) => server.ca_pem_path(),
        }
    }

    fn process_id(&self) -> u32 {
        match self {
            Self::Ergo(server) => server.process_id(),
            Self::InspIrcd(server) => server.process_id(),
            Self::Solanum(server) => server.process_id(),
            Self::Soju(server) => server.process_id(),
            Self::UnrealIrcd(server) => server.process_id(),
        }
    }

    fn bouncer_metadata(&self) -> (String, String, String, String) {
        if let Self::Soju(server) = self {
            let (account, password, config, upstream) = server.fixture_metadata();
            let config = config.display().to_string();
            let upstream = upstream.to_string();
            return (account.into(), password.into(), config, upstream);
        }
        (String::new(), String::new(), String::new(), String::new())
    }
}

#[cfg(not(target_family = "wasm"))]
fn main() -> io::Result<()> {
    // The runner helper owns the fixture outside every test process.
    let server = Server::start(DeploymentProfile::from_env()?)?;
    if let Some(path) = env::var_os("PAPO_TEST_ERGO_PID_FILE") {
        fs::write(path, server.process_id().to_string())?;
    }
    let (plaintext, tls, websocket, secure_websocket) = server.endpoints();
    if let (Ok(account), Ok(password)) = (
        env::var("PAPO_BROWSER_ACCOUNT"),
        env::var("PAPO_BROWSER_PASSWORD"),
    ) {
        register_account_at(
            plaintext.parse().map_err(io::Error::other)?,
            &account,
            &password,
        )?;
    }
    println!("{plaintext}\n{tls}\n{websocket}\n{secure_websocket}");
    println!("{}", server.ca_pem_path().display());
    let (bouncer_user, bouncer_password, bouncer_config, upstream) = server.bouncer_metadata();
    println!("{bouncer_user}");
    println!("{bouncer_password}");
    println!("{bouncer_config}");
    println!("{upstream}");
    io::stdout().flush()?;
    io::copy(&mut io::stdin(), &mut io::sink())?;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn main() {}
