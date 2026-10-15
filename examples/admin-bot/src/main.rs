//! Run an authenticated command bot with account-based operator access.

use std::{env, error::Error, io, time::Duration};

use papo::{
    Client, Error as PapoError, Event, Events, Sasl,
    bot::{BotConfig, Command, ReplyRateLimiter},
};
use papo_proto::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let nick = required("IRC_NICK")?;
    let password_file = required("IRC_PASSWORD_FILE")?;
    let channels = required("IRC_CHANNELS")?;
    let admins = required("IRC_ADMINS")?;
    let password = std::fs::read_to_string(password_file)?;

    let mut builder = Client::builder(&server)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password.trim_end_matches(['\r', '\n'])))
        .require_sasl(true)
        .bot()
        .channels(channels.split(',').map(str::trim));
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_file(ca_file)?;
    }
    let (client, events) = builder.connect().await?;
    let admins: Vec<_> = admins
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    let config =
        BotConfig::new("!").reply_limiter(ReplyRateLimiter::new(2, Duration::from_secs(5)));
    run(client, events, &config, &admins).await?;
    Ok(())
}

async fn run(
    client: Client,
    mut events: Events,
    config: &BotConfig,
    admins: &[String],
) -> Result<(), PapoError> {
    while let Some(event) = events.next().await {
        handle(&event, config, admins).await?;
    }
    drop(client);
    Ok(())
}

async fn handle(event: &Event, config: &BotConfig, admins: &[String]) -> Result<(), PapoError> {
    let Some(command) = Command::parse(event, config) else {
        return Ok(());
    };
    match command.name() {
        "ping" => {
            command.reply("pong").await?;
        }
        "echo" if !command.args().is_empty() => {
            command.reply(&command.args().join(" ")).await?;
        }
        "echo" => {
            command
                .private_notice("expected at least one command argument")
                .await?;
        }
        "op" if command.args().len() != 1 => {
            command
                .private_notice("expected exactly one command argument")
                .await?;
        }
        "op" if !command.account_in(admins) => {
            command
                .private_notice("permission denied: authenticated account required")
                .await?;
        }
        "op" => {
            let Some(channel) = command.channel() else {
                command
                    .private_notice("op requires a channel command")
                    .await?;
                return Ok(());
            };
            command
                .client()
                .send(Message::new(
                    "MODE",
                    [channel, "+o", command.args()[0].as_str()],
                ))
                .await?;
        }
        _ => {}
    }
    Ok(())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
