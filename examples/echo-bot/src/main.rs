//! Echo live IRC messages with modern reply, reaction, and typing tags.

use std::{env, error::Error, io};

use papo::{
    Client, Endpoint, Event, MessageKind, Sasl, Typing,
    bot::{BotConfig, Command},
};

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
        .autojoin(channels.split(',').map(str::trim));
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_pem(std::fs::read(ca_file)?);
    }
    let (client, mut events) = builder.connect().await?;
    let config = BotConfig::new("!");
    while let Some(event) = events.next().await {
        handle(&client, &event, &config).await?;
    }
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

async fn handle(client: &Client, event: &Event, config: &BotConfig) -> Result<(), papo::Error> {
    let Event::Message(message) = event else {
        return Ok(());
    };
    if !message.is_live() || message.kind() != MessageKind::Privmsg {
        return Ok(());
    }
    if message.text().contains('\n') {
        let text = message.text();
        let summary = format!(
            "got {} lines, {} bytes",
            text.split('\n').count(),
            text.len()
        );
        client.reply(message, &summary).await?;
        client
            .typing(message.reply_target(), Typing::Active)
            .await?;
        let reply = client.reply(message, text).await;
        client.typing(message.reply_target(), Typing::Done).await?;
        reply?;
        return Ok(());
    }

    let Some(command) = Command::parse(event, config) else {
        return Ok(());
    };
    match command.name() {
        "echo" if command.args().is_empty() => {
            command.private_notice("expected text after !echo").await?;
        }
        "echo" => {
            command.reply(&command.args().join(" ")).await?;
        }
        "react" => {
            let id = command.message().id().ok_or(papo::Error::InvalidInput(
                papo::InvalidInput::MessageReference,
            ))?;
            command.client().react(&id, "👍").await?;
        }
        _ => {}
    }
    Ok(())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
