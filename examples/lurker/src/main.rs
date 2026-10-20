//! Print channel membership and live state changes.

use std::{collections::BTreeSet, env, error::Error, io, io::Write as _};

use papo::{Account, Client, Endpoint, Event, Sasl};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = env::var("IRC_SERVER")?;
    let nick = env::var("IRC_NICK")?;
    let password_file = env::var("IRC_PASSWORD_FILE")?;
    let channels = env::var("IRC_CHANNELS")?;
    let password = std::fs::read_to_string(password_file)?;
    let mut builder = Client::builder(endpoint(server)?)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password.trim_end_matches(['\r', '\n'])))
        .autojoin(channels.split(',').map(str::trim));
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_pem(std::fs::read(ca_file)?);
    }
    let (client, mut events) = builder.connect().await?;
    let mut synced = BTreeSet::new();
    while let Some(event) = events.next().await {
        match event {
            Event::ChannelSynced(papo::event::ChannelSynced { channel, .. }) => {
                println!("synced {channel}: {}", members(&client, &channel));
                synced.insert(channel);
            }
            Event::Joined(papo::event::Joined { channel, nick, .. })
                if synced.contains(&channel) =>
            {
                println!("join {channel} {nick}");
            }
            Event::Parted(event) => {
                let reason = event.reason.as_deref().unwrap_or("-");
                println!("part {} {}: {reason}", event.channel, event.nick);
            }
            Event::Quit(mut event) => {
                event.channels.sort();
                let channels = event
                    .channels
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join(",");
                let reason = event.reason.as_deref().unwrap_or("-");
                println!("quit {} [{channels}]: {reason}", event.nick);
            }
            Event::NickChanged(event) => println!("nick {} -> {}", event.old, event.new),
            Event::ModeChanged(event) => {
                let changes = event
                    .changes
                    .into_iter()
                    .map(|change| {
                        let sign = if change.set { '+' } else { '-' };
                        format!("{sign}{}={}", change.mode, change.arg.unwrap_or_default())
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("mode {} by {}: {changes}", event.target, event.by);
            }
            Event::TopicChanged(event) => {
                let topic = event.topic.as_deref().unwrap_or("-");
                println!("topic {} by {}: {topic}", event.channel, event.by);
            }
            Event::AccountChanged(event) => {
                println!(
                    "account {}: {}",
                    event.nick,
                    event.account.as_deref().unwrap_or("*")
                );
            }
            Event::AwayChanged(event) => {
                let message = event.message.as_deref().unwrap_or("back");
                println!("away {}: {message}", event.nick);
            }
            _ => {}
        }
        io::stdout().flush()?;
    }
    Ok(())
}

fn endpoint(host: String) -> Result<Endpoint, Box<dyn Error>> {
    let port = env::var("IRC_PORT").map_or(Ok(6697), |value| value.parse())?;
    Ok(if env::var_os("IRC_PLAINTEXT").is_some() {
        Endpoint::plaintext(host, port)
    } else {
        Endpoint::tls(host, port)
    })
}

fn members(client: &Client, channel: &str) -> String {
    let state = client.state();
    let mut members = state.channel(channel).map_or_else(Vec::new, |joined| {
        joined
            .members()
            .map(|member| {
                let account = state
                    .user(member.nick())
                    .map_or("?", |user| account(user.account()));
                format!(
                    "{}{}[account={account}]",
                    member.prefixes().iter().collect::<String>(),
                    member.nick()
                )
            })
            .collect::<Vec<_>>()
    });
    members.sort();
    members.join(" ")
}

fn account(account: &Account) -> &str {
    match account {
        Account::LoggedIn(name) => name,
        Account::LoggedOut => "*",
        _ => "?",
    }
}
