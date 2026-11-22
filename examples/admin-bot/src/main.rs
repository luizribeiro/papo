//! Authorize an operator command by account.

use std::{env, error::Error};

use papo::{AccountStatus, Client, Event};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: admin-bot URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-admin".into());
    let admin = env::var("IRC_ADMIN")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo-admin".into());
    let (client, mut events) = Client::builder(url, nick)
        .bot_mode(true)
        .autojoin([&channel])
        .connect()
        .await?;
    while let Some((event, _)) = events.next().await {
        let Event::Message(message) = event else {
            continue;
        };
        let Some(nick) = message.text.strip_prefix("!op ") else {
            continue;
        };
        let is_admin = matches!(
            message.actor_account(),
            AccountStatus::LoggedIn(account) if account == admin.as_str()
        );
        let reason = if !is_admin {
            "permission denied".to_owned()
        } else if let Err(error) = op(&client, &channel, nick).await {
            error.to_string()
        } else {
            continue;
        };
        if let Err(error) = client
            .reply(&message, format!("cannot op {nick}: {reason}"))
            .await
        {
            eprintln!("could not reply: {error}");
        }
    }
    Ok(())
}

async fn op(client: &Client, channel: &str, nick: &str) -> Result<(), papo::Error> {
    client.set_modes(channel).op(nick).await?;
    Ok(())
}
