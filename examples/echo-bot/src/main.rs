//! Echo messages with replies, reactions, and multi-line delivery.

use std::{env, error::Error};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-echo".into());
    let (client, mut events) = Client::builder(server)
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
        let result = if message.text == "!react" {
            client.react(&*message, "👍").await
        } else {
            client.reply(&message, &message.text).await.map(drop)
        };
        if let Err(error) = result {
            eprintln!("could not answer: {error}");
        }
    }
    Ok(())
}
