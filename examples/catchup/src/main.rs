//! Print unread channel history, then advance each read marker.

use std::{collections::BTreeSet, env, error::Error, io, time::UNIX_EPOCH};

use papo::{Client, Endpoint, Event, MessageKind, Sasl, Source};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = required("IRC_SERVER")?;
    let nick = required("IRC_NICK")?;
    let password_file = required("IRC_PASSWORD_FILE")?;
    let channels = required("IRC_CHANNELS")?
        .split(',')
        .map(str::trim)
        .filter(|channel| !channel.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let password = std::fs::read_to_string(password_file)?;
    let mut builder = Client::builder(endpoint(server)?)
        .nick(&nick)
        .sasl(Sasl::plain(&nick, password.trim_end_matches(['\r', '\n'])))
        .request_caps([
            "draft/chathistory",
            "draft/read-marker",
            "labeled-response",
            "batch",
            "server-time",
        ])
        .autojoin(&channels);
    if let Some(ca_file) = env::var_os("IRC_CA_FILE") {
        builder = builder.tls_ca_pem(std::fs::read(ca_file)?);
    }
    let (client, mut events) = builder.connect().await?;
    wait_for_channels(&mut events, &channels).await?;

    for channel in channels {
        let mut after = client.read_marker(&channel).await?.unwrap_or(UNIX_EPOCH);
        let mut newest = None;
        loop {
            let page = client.history(&channel).after(after).limit(100).await?;
            for message in page.messages() {
                if matches!(
                    message.kind(),
                    MessageKind::Privmsg | MessageKind::Notice | MessageKind::Action
                ) && !matches!(
                    message.source(),
                    Some(Source::User { ref nick, .. }) if nick == "HistServ"
                ) {
                    println!("{channel} {}", message.text());
                }
                after = message.time();
                newest = Some(after);
            }
            if page.end || page.messages().next().is_none() {
                break;
            }
        }
        if let Some(newest) = newest {
            client.mark_read(&channel, newest).await?;
        }
    }
    client.quit(Some("history caught up")).await?;
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

async fn wait_for_channels(
    events: &mut papo::Events,
    channels: &[String],
) -> Result<(), papo::Error> {
    let mut pending = channels.iter().cloned().collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        match events.next().await {
            Some(Event::ChannelSynced(papo::event::ChannelSynced { channel, .. })) => {
                _ = pending.remove(&channel);
            }
            Some(Event::Disconnected(_)) | None => {
                return Err(papo::Error::Disconnected(papo::Disconnected::new(
                    papo::DisconnectCause::ConnectionLost,
                    None,
                )));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is required")))
}
