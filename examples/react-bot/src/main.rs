//! React to live messages.

use std::{env, error::Error};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: react-bot URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-react".into());
    let (client, mut events) = Client::builder(url)
        .nick(nick)
        .autojoin([CHANNEL])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        let Event::Message(message) = event else {
            continue;
        };
        if !message.is_live() {
            continue;
        }
        let _ = client.react(&message, "👍").await;
    }
    Ok(())
}
