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
        Client::builder(bouncer_url.clone(), &nick)
            .sasl(Sasl::plain(&account, password.clone()))
            .allow_insecure_auth(bouncer_url.starts_with("irc://"))
            .enable(Feature::Bouncer)
    };
    let (manager, mut manager_events) = builder().connect().await?;
    for network in manager.bouncer().networks().await? {
        print_network("network", &network);
    }

    let config = NetworkConfig::from_url(&upstream_url)?
        .name("second")
        .nick(&nick);
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

    let (_attached, mut events) = manager
        .bouncer()
        .network(&id)
        .autojoin([CHANNEL])
        .connect()
        .await?;
    while let Some(event) = events.next().await {
        if let Event::ChannelSynced(synced) = event {
            println!("attached {id} {}", synced.channel);
            return Ok(());
        }
    }
    Ok(())
}

fn print_network(prefix: &str, network: &papo::BouncerNetwork) {
    let state = network
        .state
        .map_or_else(|| "unknown".to_owned(), |state| state.to_string());
    println!(
        "{prefix} {}: {} {} {}",
        network.id,
        network.name.as_deref().unwrap_or("-"),
        network.host.as_deref().unwrap_or("-"),
        state
    );
}
