//! Run an authenticated command bot with account-based operator access.

use std::{env, error::Error, io, time::Duration};

use papo::{
    Client, Sasl,
    bot::{Admins, Args, Bot, Msg, ReplyRateLimiter, RequireAccount},
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
    Bot::new(client)
        .prefix("!")
        .admins(Admins::new(
            admins.split(',').map(str::trim).filter(|name| !name.is_empty()),
        ))
        .reply_limiter(ReplyRateLimiter::new(2, Duration::from_secs(5)))
        .command("ping", |msg: Msg| async move { msg.reply("pong").await })
        .command(
            "echo",
            |msg: Msg, Args(words): Args<Vec<String>>| async move {
                msg.reply(&words.join(" ")).await
            },
        )
        .command(
            "op",
            |msg: Msg, Args(nick): Args<String>, _: RequireAccount<Admins>| async move {
                let Some(channel) = msg.channel().map(str::to_owned) else {
                    return msg.private_notice("op requires a channel command").await;
                };
                msg.client()
                    .send(Message::new("MODE", [channel, "+o".to_owned(), nick]))
                    .await
            },
        )
        .run(events)
        .await?;
    Ok(())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
