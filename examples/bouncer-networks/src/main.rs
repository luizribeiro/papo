//! Manage several upstream networks through one bouncer account.

use std::{env, error::Error};

use papo::{Client, Event, Feature, NetworkConfig, NetworkState, Sasl};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let bouncer_url = args
        .next()
        .ok_or("usage: bouncer-networks BOUNCER UPSTREAM")?;
    let upstream_url = args
        .next()
        .ok_or("usage: bouncer-networks BOUNCER UPSTREAM")?;
    let nick = env::var("IRC_NICK")?;
    let account = env::var("IRC_ACCOUNT")?;
    let password = env::var("IRC_PASSWORD")?;

    let builder = || {
        Client::builder(bouncer_url.clone())
            .nick(&nick)
            .sasl(Sasl::plain(&account, password.clone()))
            .allow_insecure_auth(bouncer_url.starts_with("irc://"))
            .enable(Feature::Bouncer)
    };
    let (manager, mut manager_events) = builder().connect().await?;
    for network in manager.bouncer().networks().await? {
        print_network("network", &network);
    }

    let config = network_config(&upstream_url, manager.state().nickname(&nick))?;
    let id = manager.bouncer().add(config).await?;
    while let Some(event) = manager_events.next().await {
        if let Event::BouncerNetworkChanged(changed) = event
            && changed.id == id
            && let Some(network) = changed.network
        {
            print_network("changed", &network);
            if network.state == Some(NetworkState::Connected) {
                break;
            }
        }
    }

    let (attached, mut events) = builder()
        .bouncer_network(id.clone())
        .autojoin([CHANNEL])
        .connect()
        .await?;
    assert_eq!(attached.state().bouncer_network(), Some(&id));
    while let Some(event) = events.next().await {
        if let Event::ChannelSynced(synced) = event {
            println!("attached {id} {}", synced.channel);
            return Ok(());
        }
    }
    Ok(())
}

fn network_config(url: &str, nick: papo::Nick) -> Result<NetworkConfig, Box<dyn Error>> {
    let (scheme, address) = url.split_once("://").ok_or("upstream URL needs a scheme")?;
    let (host, port) = address
        .rsplit_once(':')
        .ok_or("upstream URL needs a port")?;
    let tls = match scheme {
        "irc" => false,
        "ircs" => true,
        _ => return Err("upstream URL must use irc:// or ircs://".into()),
    };
    Ok(NetworkConfig::new(host.trim_matches(['[', ']']))
        .port(port.parse()?)
        .tls(tls)
        .name("second")
        .nickname(nick))
}

fn print_network(prefix: &str, network: &papo::BouncerNetwork) {
    println!(
        "{prefix} {}: {} {} {}",
        network.id,
        network.name.as_deref().unwrap_or("-"),
        network.host.as_deref().unwrap_or("-"),
        state_name(network.state)
    );
}

const fn state_name(state: Option<NetworkState>) -> &'static str {
    match state {
        Some(NetworkState::Connected) => "Connected",
        Some(NetworkState::Connecting) => "Connecting",
        Some(NetworkState::Disconnected) => "Disconnected",
        _ => "Unknown",
    }
}
