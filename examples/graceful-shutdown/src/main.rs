//! Send `QUIT` before exiting on SIGINT or SIGTERM.

use std::{env, error::Error, future::Future, io};

use papo::Client;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: graceful-shutdown URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-client".into());
    let shutdown = shutdown_signal()?;
    let (client, mut events) = Client::builder(url, nick).connect().await?;

    tokio::select! {
        () = shutdown => {
            eprintln!("shutting down");
            if let Err(error) = client.quit().reason("shutting down").await {
                eprintln!("could not quit: {error}");
            }
        }
        () = async { while events.next().await.is_some() {} } => return Ok(()),
    }
    while events.next().await.is_some() {}
    Ok(())
}

/// Listens from the moment it is called, so a signal sent while connecting is not lost.
#[cfg(unix)]
fn shutdown_signal() -> io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    })
}

#[cfg(windows)]
fn shutdown_signal() -> io::Result<impl Future<Output = ()>> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    Ok(async move {
        ctrl_c.recv().await;
    })
}

#[cfg(target_os = "wasi")]
fn shutdown_signal() -> io::Result<impl Future<Output = ()>> {
    Ok(std::future::pending())
}
