//! Print unread channel history and advance its read marker.

use std::{env, error::Error, time::UNIX_EPOCH};

use papo::{Client, Event, Sasl};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-catchup".into());
    let password = env::var("IRC_PASSWORD")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo".into());
    let (client, mut events) = Client::builder(server)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password))
        .autojoin([&channel])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        if matches!(event, Event::ChannelSynced(ref synced) if synced.channel == channel.as_str()) {
            break;
        }
    }
    let marker = client
        .fetch_read_marker(&channel)
        .await?
        .unwrap_or(UNIX_EPOCH);
    let history = client.history(&channel).after(marker).limit(100).await?;
    let mut newest = None;
    for message in history.messages() {
        println!("{channel} {}", message.text);
        newest = Some(message.time());
    }
    if let Some(time) = newest {
        client.mark_read(&channel, time).await?;
    }
    Ok(())
}
