use std::{io, time::Duration};

use papo::{
    Client, DisconnectCause, Endpoint, Error, Event, Feature, InvalidInput, Reconnect, Sasl,
};
use papo_conformance_macros::scenario;

use crate::scenarios::support::message_text;
use crate::{Peer, Scenario};

async fn provisioned_control(
    s: &Scenario,
    prefix: &str,
) -> papo_testing::Result<(Peer, papo::NetworkId)> {
    let bouncer = s
        .server
        .bouncer()
        .expect("the soju profile has bouncer configuration");
    let network_name = papo_test_server::unique(prefix);
    bouncer.provision_network(&network_name)?;
    let address = s.server.plaintext_addr();
    let control = s
        .connect_with(
            Client::builder(Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap())
                .nick(&network_name)
                .sasl(Sasl::plain(bouncer.username(), bouncer.password()))
                .allow_insecure_auth(true)
                .enable(Feature::Bouncer),
        )
        .await
        .map_err(|error| io::Error::other(format!("management connection failed: {error:?}")))?;
    let id = control
        .bouncer()
        .networks()
        .await
        .map_err(|error| io::Error::other(format!("network list failed: {error:?}")))?
        .into_iter()
        .find(|network| network.name.as_deref() == Some(network_name.as_str()))
        .expect("the provisioned network is listed")
        .id;
    Ok((control, id))
}

/// refuses a message reference from another connection bound to the same bouncer network
#[scenario(
    requires(caps = ["soju.im/bouncer-networks"]),
    covers = ["security.origin.bound-client-wire"]
)]
async fn bound_client_reference(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile is not the managed soju bouncer");
    }
    let bouncer = s
        .server
        .bouncer()
        .expect("the soju profile has bouncer configuration");
    let (control, id) = provisioned_control(&s, "origin-network").await?;
    let channel = s.channel_name();
    let mut first = s
        .connect_with(control.bouncer().network(&id).autojoin([&channel]))
        .await
        .map_err(|error| io::Error::other(format!("first bound connection failed: {error:?}")))?;
    let mut second = s
        .connect_with(control.bouncer().network(&id).autojoin([&channel]))
        .await?;
    assert_eq!(first.state().bouncer_network(), Some(&id));
    assert_eq!(second.state().bouncer_network(), Some(&id));
    first.wait_for(Event::as_channel_synced).await?;
    second.wait_for(Event::as_channel_synced).await?;

    let upstream = bouncer.upstream_addr();
    let mut sender = s
        .connect_with(
            Client::builder(
                Endpoint::plaintext(upstream.ip().to_string(), upstream.port()).unwrap(),
            )
            .nick(papo_test_server::unique("origin-sender"))
            .allow_insecure_auth(true),
        )
        .await?;
    sender.join(&channel).await?;
    sender.privmsg(&channel, "origin boundary probe").await?;
    let first_message = first
        .wait_for(|event| message_text(event, "origin boundary probe"))
        .await?;
    let second_message = second
        .wait_for(|event| message_text(event, "origin boundary probe"))
        .await?;
    assert_ne!(first_message.origin(), second_message.origin());
    assert_ne!(first_message.conversation, second_message.conversation);
    assert_eq!(first_message.conversation, channel.as_str());
    assert_eq!(second_message.conversation, channel.as_str());

    let error = first
        .reply(&second_message, "must remain local")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput(InvalidInput::ForeignReference)
    ));
    sender
        .expect_none(|event| message_text(event, "must remain local"))
        .await?;
    Ok(())
}

/// stops reconnecting when a selected bouncer network is deleted
#[scenario(
    requires(caps = ["soju.im/bouncer-networks"]),
    covers = ["security.bouncer.deleted-network-fatal"]
)]
async fn deleted_bound_network_stops_reconnecting(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile is not the managed soju bouncer");
    }
    let (control, id) = provisioned_control(&s, "deleted-network").await?;
    let (_bound, mut events) = control
        .bouncer()
        .network(&id)
        .reconnect(
            Reconnect::exponential()
                .initial_delay(Duration::from_millis(10))
                .max_delay(Duration::from_millis(10)),
        )
        .connect()
        .await?;
    while !matches!(events.next().await, Some(Event::Connected(_))) {}

    control.bouncer().remove(&id).await?;
    let reconnecting = loop {
        match events.next().await {
            Some(Event::Disconnected(disconnected)) => break disconnected,
            Some(_) => {}
            None => panic!("bound client stopped before reporting the deletion"),
        }
    };
    assert!(reconnecting.is_reconnecting());
    let stopped = loop {
        match events.next().await {
            Some(Event::Disconnected(disconnected)) => break disconnected,
            Some(_) => {}
            None => panic!("bound client stopped without the binding error"),
        }
    };
    assert!(stopped.reconnect_in.is_none());
    assert!(matches!(
        &stopped.cause,
        DisconnectCause::Connect(error)
            if std::error::Error::source(error)
                .is_some_and(|source| source.to_string().contains("selected network ID"))
    ));
    Ok(())
}
