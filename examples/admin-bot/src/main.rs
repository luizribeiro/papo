//! Parse bot commands and authorize an operator command by account.

use std::{env, error::Error};

use papo::{Client, Event};
use papo_bot::{BotConfig, Command};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-admin".into());
    let admin = env::var("IRC_ADMIN")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo-admin".into());
    let (client, mut events) = Client::builder(server)
        .nick(nick)
        .bot(true)
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
        match (command.name(), command.args()) {
            ("ping", []) => reply(&command, "pong").await,
            ("op", [nick]) if command.account_in([&admin]) => {
                if let Err(refusal) = command.client().mode(&channel).op(nick).await {
                    reply(&command, format!("cannot op {nick}: {refusal}")).await;
                }
            }
            ("op", [nick]) => {
                reply(&command, format!("cannot op {nick}: permission denied")).await;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn reply(command: &Command, text: impl Into<String>) {
    if let Err(error) = command.reply(text).await {
        eprintln!("could not reply: {error}");
    }
}
