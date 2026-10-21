//! Parse bot commands and authorize an operator command by account.

use std::{env, error::Error};

use papo::{Client, Event, bot::BotConfig};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    run().await
}

async fn run() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-admin".into());
    let admin = env::var("IRC_ADMIN")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo-admin".into());
    let (client, mut events) = Client::builder(server)
        .nick(nick)
        .bot()
        .autojoin([&channel])
        .connect()
        .await?;
    let commands = BotConfig::new("!");

    while let Some(event) = events.next().await {
        let Event::Message(message) = event else {
            continue;
        };
        let Some(command) = commands.command(&client, &message) else {
            continue;
        };
        let result = match (command.name(), command.args()) {
            ("ping", []) => command.reply("pong").await.map(drop),
            ("whois", [nick]) => {
                let (command, nick) = (command.clone(), nick.clone());
                tokio::spawn(async move {
                    let text = command.client().whois(&nick).await.map_or_else(
                        |error| format!("cannot look up {nick}: {error}"),
                        |user| format!("{} is connected", user.nick),
                    );
                    let _ = command.reply(text).await;
                });
                Ok(())
            }
            ("op", [nick]) if command.account_in([&admin]) => {
                match command.client().mode(&channel).op(nick).await {
                    Ok(()) => Ok(()),
                    Err(refusal) => command
                        .reply(format!("cannot op {nick}: {refusal}"))
                        .await
                        .map(drop),
                }
            }
            ("op", [nick]) => command
                .reply(format!("cannot op {nick}: permission denied"))
                .await
                .map(drop),
            _ => Ok(()),
        };
        if let Err(error) = result {
            eprintln!("could not answer command: {error}");
        }
    }
    Ok(())
}
