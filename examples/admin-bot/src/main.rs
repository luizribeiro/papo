//! Run an authenticated command bot with account-based operator access.

use std::{env, error::Error, io, time::Duration};

use papo::{
    Client, Endpoint, Error as PapoError, Event, Sasl,
    bot::{BotConfig, ReplyRateLimiter},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let nick = required("IRC_NICK")?;
    let password_file = required("IRC_PASSWORD_FILE")?;
    let channels = required("IRC_CHANNELS")?;
    let admins = required("IRC_ADMINS")?;
    let password = std::fs::read_to_string(password_file)?;

    let mut builder = Client::builder(endpoint(server)?)
        .nick(papo::Casemapping::default().nick(&nick))
        .sasl(Sasl::plain(&nick, password.trim_end_matches(['\r', '\n'])))
        .bot()
        .autojoin(channels.split(',').map(str::trim));
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_pem(std::fs::read(ca_file)?);
    }
    let (client, mut events) = builder.connect().await?;
    let admins: Vec<_> = admins
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    let config =
        BotConfig::new("!").reply_limiter(ReplyRateLimiter::new(2, Duration::from_secs(5))?);
    while let Some(event) = events.next().await {
        handle(&client, &event, &config, &admins).await?;
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

async fn handle(
    client: &Client,
    event: &Event,
    config: &BotConfig,
    admins: &[String],
) -> Result<(), PapoError> {
    let Some(command) = event
        .as_message()
        .and_then(|message| config.command(client, message))
    else {
        return Ok(());
    };
    match command.name() {
        "ping" => {
            let _ = command.reply("pong").await?;
        }
        "echo" if !command.args().is_empty() => {
            let _ = command.reply(command.args().join(" ")).await?;
        }
        "echo" => {
            let _ = command
                .private_notice("expected at least one command argument")
                .await?;
        }
        "op" if command.args().len() != 1 => {
            let _ = command
                .private_notice("expected exactly one command argument")
                .await?;
        }
        "op" if !command.account_in(admins) => {
            let _ = command
                .private_notice("permission denied: authenticated account required")
                .await?;
        }
        "op" => {
            let Some(channel) = command.channel() else {
                let _ = command
                    .private_notice("op requires a channel command")
                    .await?;
                return Ok(());
            };
            command
                .client()
                .mode(channel)
                .op(command
                    .client()
                    .state()
                    .casemapping()
                    .nick(command.args()[0].as_str()))
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
