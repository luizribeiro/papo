//! Send `QUIT` before exiting on SIGINT or SIGTERM.

use std::{env, error::Error};

#[cfg(not(target_os = "wasi"))]
use std::io;

use papo::Client;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-client".into());
    let (client, mut events) = Client::builder(server).nick(nick).connect().await?;

    #[cfg(not(target_os = "wasi"))]
    tokio::select! {
        signal = shutdown_signal() => {
            signal?;
            eprintln!("shutting down");
            if let Err(error) = client.quit("shutting down").await {
                eprintln!("could not quit: {error}");
            }
        }
        () = async { while events.next().await.is_some() {} } => {}
    }

    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    {
        let _client = client;
        while events.next().await.is_some() {}
    }
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(all(not(unix), not(target_os = "wasi")))]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}
