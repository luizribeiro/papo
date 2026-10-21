//! Print unread channel history and advance its read marker.

use std::{env, error::Error, time::UNIX_EPOCH};

use papo::{Client, Event, Sasl};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: catchup URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-catchup".into());
    let password = env::var("IRC_PASSWORD")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo".into());
    let (client, mut events) = Client::builder(url)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password))
        .autojoin([&channel])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        if !matches!(event, Event::ChannelSynced(ref synced) if synced.channel == channel.as_str())
        {
            continue;
        }
        if let Err(error) = catch_up(&client, &channel).await {
            eprintln!("could not catch up: {error}");
        }
        return Ok(());
    }
    Ok(())
}

async fn catch_up(client: &Client, channel: &str) -> Result<(), papo::Error> {
    let marker = client
        .fetch_read_marker(channel)
        .await?
        .unwrap_or(UNIX_EPOCH);
    let history = client.history(channel).after(marker).limit(100).await?;
    let mut newest = None;
    for message in history.messages() {
        println!("{channel} {}", message.text);
        newest = Some(message.time());
    }
    if let Some(time) = newest {
        client.mark_read(channel, time).await?;
    }
    Ok(())
}
