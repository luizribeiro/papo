//! Runs a test server until its standard input closes.

#[cfg(not(target_family = "wasm"))]
use std::io::{self, Write};

#[cfg(not(target_family = "wasm"))]
use papo_test_server::TestServer;

#[cfg(not(target_family = "wasm"))]
fn main() -> io::Result<()> {
    let server = TestServer::start()?;
    println!("{}", server.plaintext_addr());
    println!("{}", server.tls_addr());
    println!("{}", server.ca_pem_path().display());
    io::stdout().flush()?;
    io::copy(&mut io::stdin(), &mut io::sink())?;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn main() {}
