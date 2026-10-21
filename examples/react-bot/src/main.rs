//! React to live messages.

use std::{env, error::Error};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-react".into());
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
        if let Err(error) = client.react(&message, "👍").await {
            eprintln!("could not react: {error}");
        }
    }
    Ok(())
}
