//! Print unread channel history and advance its read marker.

use std::{env, error::Error, time::UNIX_EPOCH};

use papo::{Client, Event, Sasl, Secret};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: catchup URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-catchup".into());
    let password = env::var("IRC_PASSWORD")?;
    let channel = env::var("IRC_CHANNEL").unwrap_or_else(|_| "#papo".into());
    let mut builder = Client::builder(url, &nick)
        .sasl(Sasl::plain(&nick, Secret::new(password)))
        .autojoin([&channel]);
    if env::var_os("IRC_ALLOW_INSECURE_AUTH").is_some() {
        builder = builder.allow_insecure_auth(true);
    }
    let (client, mut events) = builder.connect().await?;

    while let Some(event) = events.next().await {
        if !matches!(event, Event::ChannelSynced(ref synced) if synced.channel == channel.as_str())
        {
            continue;
        }
        if let Err(error) = catch_up(&client, &channel).await {
            eprintln!("could not catch up: {error}");
        }
        client.quit("catchup complete").await?;
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
    for message in history.chat_messages() {
        println!("{channel} {}", message.text);
        newest = message.server_time;
    }
    if let Some(time) = newest {
        client.mark_read(channel, time).await?;
    }
    Ok(())
}
