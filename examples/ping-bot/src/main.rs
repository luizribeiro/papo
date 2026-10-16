//! Answer live IRC `!ping` messages with threaded `pong` replies.

use std::{env, error::Error, io};

use papo::{Client, Endpoint, Event, Sasl};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let nick = required("IRC_NICK")?;
    let password_file = required("IRC_PASSWORD_FILE")?;
    let channels = required("IRC_CHANNELS")?;
    let password = std::fs::read_to_string(password_file)?;

    let mut builder = Client::builder(endpoint(server)?)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password.trim_end_matches(['\r', '\n'])))
        .bot()
        .channels(channels.split(',').map(str::trim));
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_file(ca_file)?;
    }
    let (client, events) = builder.connect().await?;
    run(client, events).await?;
    Ok(())
}

fn endpoint(host: String) -> Result<Endpoint, Box<dyn Error>> {
    let port = env::var("IRC_PORT").map_or(Ok(6697), |value| value.parse())?;
    Ok(if env::var_os("IRC_PLAINTEXT").is_some() {
        Endpoint::plaintext(host, port)
    } else {
        Endpoint::tls(host, port)
    })
}

#[cfg(not(target_os = "wasi"))]
async fn run(client: Client, mut events: papo::Events) -> Result<(), papo::Error> {
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            signal = &mut shutdown => {
                signal?;
                client.quit(Some("shutting down")).await?;
                return Ok(());
            }
            event = events.next() => match event {
                Some(event) => handle(event).await?,
                None => return Ok(()),
            },
        }
    }
}

// WASI has no process signals, so the connection lifetime controls shutdown.
#[cfg(all(target_os = "wasi", not(target_env = "p1")))]
async fn run(_client: Client, mut events: papo::Events) -> Result<(), papo::Error> {
    while let Some(event) = events.next().await {
        handle(event).await?;
    }
    Ok(())
}

async fn handle(event: Event) -> Result<(), papo::Error> {
    match event {
        Event::Registered(welcome) => eprintln!("connected as {}", welcome.nick),
        Event::Reconnected(welcome) => eprintln!("reconnected as {}", welcome.nick),
        Event::Disconnected { reason, will_retry } => {
            eprintln!("disconnected: {reason}; reconnecting: {will_retry}");
        }
        Event::Message(message) if message.is_live() && message.text() == "!ping" => {
            message.reply("pong").await?;
        }
        _ => {}
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

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
