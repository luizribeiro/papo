//! Answer live IRC `!ping` messages with threaded `pong` replies.

use std::{env, error::Error};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: ping-bot URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-ping".into());
    let (client, mut events) = Client::builder(url)
        .nick(nick)
        .bot(true)
        .autojoin([CHANNEL])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        if let Event::Message(message) = event
            && message.text == "!ping"
            && let Err(error) = client.reply(&message, "pong").await
        {
            eprintln!("could not reply: {error}");
        }
    }
    Ok(())
}
