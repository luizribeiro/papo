//! Runs a test server until its standard input closes.

#[cfg(not(target_family = "wasm"))]
use std::{
    env, fs,
    io::{self, Write},
};

#[cfg(not(target_family = "wasm"))]
use papo_test_server::TestServer;

#[cfg(not(target_family = "wasm"))]
fn main() -> io::Result<()> {
    // The runner helper owns the fixture outside every test process.
    let server = TestServer::start()?;
    if let Some(path) = env::var_os("PAPO_TEST_ERGO_PID_FILE") {
        fs::write(path, server.process_id().to_string())?;
    }
    println!("{}", server.plaintext_addr());
    println!("{}", server.tls_addr());
    println!("{}", server.ca_pem_path().display());
    io::stdout().flush()?;
    io::copy(&mut io::stdin(), &mut io::sink())?;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn main() {}
