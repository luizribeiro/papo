//! Manage several upstream networks through one bouncer account.

use std::{env, error::Error};

use papo::{Client, Event, Feature, NetworkState, Sasl, Secret};

const CHANNEL: &str = "#papo";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let (Some(bouncer_url), Some(upstream_url)) = (args.next(), args.next()) else {
        return Err("usage: bouncer-networks BOUNCER UPSTREAM".into());
    };
    let nick = env::var("IRC_NICK")?;
    let account = env::var("IRC_ACCOUNT")?;
    let password = env::var("IRC_PASSWORD")?;

    let insecure = bouncer_url.starts_with("irc://");
    let (manager, mut manager_events) = Client::builder(bouncer_url, &nick)
        .sasl(Sasl::plain(&account, Secret::new(password)))
        .allow_insecure_auth(insecure)
        .enable(Feature::Bouncer)
        .connect()
        .await?;
    for network in manager.bouncer().networks().await? {
        print_network("network", &network);
    }

    let id = manager
        .bouncer()
        .add(upstream_url)
        .name("second")
        .nick(&nick)
        .await?;
    while let Some((event, _)) = manager_events.next().await {
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

    let (attached, mut events) = manager
        .bouncer()
        .network(&id)
        .autojoin([CHANNEL])
        .connect()
        .await?;
    while let Some((event, _)) = events.next().await {
        if let Event::ChannelSynced(synced) = event {
            println!("attached {id} {}", synced.channel);
            attached.quit().await?;
            manager.quit().await?;
            return Ok(());
        }
    }
    Ok(())
}

fn print_network(prefix: &str, network: &papo::BouncerNetwork) {
    let state = network
        .state
        .map_or("unknown".into(), |state| state.to_string());
    println!(
        "{prefix} {}: {} {} {}",
        network.id,
        network.name.as_deref().unwrap_or("-"),
        network.host.as_deref().unwrap_or("-"),
        state
    );
}
