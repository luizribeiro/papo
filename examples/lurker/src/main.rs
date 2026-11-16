//! Observe channel membership through events and state snapshots.

use std::{env, error::Error, io::Write as _};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = env::args().nth(1).ok_or("usage: lurker URL")?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-lurker".into());
    let (_client, mut events) = Client::builder(url, nick)
        .autojoin([CHANNEL])
        .connect()
        .await?;

    while let Some((event, state)) = events.next().await {
        match event {
            Event::ChannelSynced(synced) => {
                let Some(channel) = state.__channel_local(&synced.channel) else {
                    continue;
                };
                let mut members = channel
                    .members()
                    .map(|member| member.nick().to_string())
                    .collect::<Vec<_>>();
                members.sort();
                println!("members {}: {}", synced.channel, members.join(" "));
            }
            Event::Joined(joined) => {
                let Some(user) = state.user(&joined.actor)? else {
                    continue;
                };
                println!("join {} {}", joined.channel, user.nick());
            }
            Event::Parted(parted) => {
                let Some(user) = parted.facts else { continue };
                println!("part {} {}", parted.channel, user.nick());
            }
            Event::Quit(quit) => {
                let Some(user) = quit.facts else { continue };
                println!("quit {}", user.nick());
            }
            Event::NickChanged(changed) => {
                println!("nick {} -> {}", changed.old, changed.new);
            }
            _ => continue,
        }
        std::io::stdout().flush()?;
    }
    Ok(())
}
