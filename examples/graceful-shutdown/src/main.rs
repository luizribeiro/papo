//! Send `QUIT` before exiting on SIGINT or SIGTERM.

use std::{env, error::Error, io};

use papo::Client;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: graceful-shutdown URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-client".into());
    let (client, mut events) = Client::builder(url, nick).connect().await?;

    tokio::select! {
        signal = shutdown_signal() => {
            signal?;
            eprintln!("shutting down");
            if let Err(error) = client.quit("shutting down").await {
                eprintln!("could not quit: {error}");
            }
        }
        () = async { while events.next().await.is_some() {} } => return Ok(()),
    }
    while events.next().await.is_some() {}
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

#[cfg(windows)]
async fn shutdown_signal() -> io::Result<()> {
    tokio::signal::ctrl_c().await
}

#[cfg(target_os = "wasi")]
fn shutdown_signal() -> impl std::future::Future<Output = io::Result<()>> {
    std::future::pending()
}
