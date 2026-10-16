//! Compile-only custom transport example for platforms without outbound sockets.

use papo::Client;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (stream, _server) = tokio::io::duplex(1024);
    let _ = Client::builder("irc://custom-transport.invalid")
        .nick("papo-custom")
        .connect_with(stream)
        .await;
}
