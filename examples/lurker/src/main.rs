//! Observe channel membership through events and state snapshots.

use std::{env, error::Error, io::Write as _};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: lurker URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-lurker".into());
    let (client, mut events) = Client::builder(url)
        .nick(nick)
        .autojoin([CHANNEL])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        match event {
            Event::ChannelSynced(synced) => {
                let state = client.state();
                let Some(channel) = state.channel(&synced.channel) else {
                    continue;
                };
                let mut members = channel
                    .members()
                    .map(|member| member.nick().to_string())
                    .collect::<Vec<_>>();
                members.sort();
                println!("members {}: {}", synced.channel, members.join(" "));
            }
            Event::Joined(joined) => println!("join {} {}", joined.channel, joined.actor.nick()),
            Event::Parted(parted) => println!("part {} {}", parted.channel, parted.actor.nick()),
            Event::Quit(quit) => println!("quit {}", quit.actor.nick()),
            Event::NickChanged(changed) => {
                println!("nick {} -> {}", changed.actor.nick(), changed.new);
            }
            _ => continue,
        }
        std::io::stdout().flush()?;
    }
    Ok(())
}
