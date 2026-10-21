//! Observe channel membership through events and state snapshots.

use std::{env, error::Error, io::Write as _};

use papo::{Client, Event};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::args()
        .nth(1)
        .map_or_else(|| env::var("IRC_SERVER"), Ok)?;
    let nick = env::var("IRC_NICK").unwrap_or_else(|_| "papo-lurker".into());
    let (client, mut events) = Client::builder(server)
        .nick(nick)
        .autojoin([CHANNEL])
        .connect()
        .await?;

    while let Some(event) = events.next().await {
        match event {
            Event::ChannelSynced(synced) => {
                let state = client.state();
                let mut members = state
                    .channel(&synced.channel)
                    .into_iter()
                    .flat_map(papo::Channel::members)
                    .map(|member| member.nick().to_string())
                    .collect::<Vec<_>>();
                members.sort();
                println!("members {}: {}", synced.channel, members.join(" "));
            }
            Event::Joined(joined) => println!("join {} {}", joined.channel, joined.nick),
            Event::Parted(parted) => println!("part {} {}", parted.channel, parted.nick),
            Event::Quit(quit) => println!("quit {}", quit.nick),
            Event::NickChanged(changed) => println!("nick {} -> {}", changed.old, changed.new),
            _ => continue,
        }
        std::io::stdout().flush()?;
    }
    Ok(())
}
