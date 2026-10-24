//! Parse bot commands and authorize an operator command by account.

use std::{env, error::Error};

use papo::{Account, Client, Event, Message};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: admin-bot URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-admin".into());
    let admin = env::var("IRC_ADMIN")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo-admin".into());
    let (client, mut events) = Client::builder(url)
        .nick(nick)
        .bot(true)
        .autojoin([&channel])
        .connect()
        .await?;
    while let Some(event) = events.next().await {
        let Event::Message(message) = event else {
            continue;
        };
        let Some(body) = command_body(&client, &message) else {
            continue;
        };
        let words = body.split_whitespace().collect::<Vec<_>>();
        match words.as_slice() {
            [command] if command.eq_ignore_ascii_case("ping") => {
                reply(&client, &message, "pong").await;
            }
            [command, nick]
                if command.eq_ignore_ascii_case("op")
                    && matches!(&message.account, Account::LoggedIn(account) if account == admin.as_str()) =>
            {
                if let Err(refusal) = client.mode(&channel).op(nick).await {
                    reply(&client, &message, format!("cannot op {nick}: {refusal}")).await;
                }
            }
            [command, nick] if command.eq_ignore_ascii_case("op") => {
                reply(
                    &client,
                    &message,
                    format!("cannot op {nick}: permission denied"),
                )
                .await;
            }
            _ => {}
        }
    }
    Ok(())
}

fn command_body<'a>(client: &Client, message: &'a Message) -> Option<&'a str> {
    if let Some(body) = message.text.strip_prefix('!') {
        return Some(body.trim_start());
    }
    let boundary = message.text.find([':', ','])?;
    let (address, body) = message.text.split_at(boundary);
    (client.nick() == address.trim()).then(|| body[1..].trim_start())
}

async fn reply(client: &Client, message: &Message, text: impl Into<String>) {
    if let Err(error) = client.reply(message, text).await {
        eprintln!("could not reply: {error}");
    }
}
