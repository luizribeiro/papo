use std::{io, time::Duration};

use papo::{
    Client, DisconnectCause, Endpoint, Error, Event, Feature, InvalidInput, NetworkConfig,
    NetworkId, NetworkState, Reconnect, Sasl, event::BouncerNetworkChanged,
};
use papo_conformance_macros::scenario;

use crate::scenarios::support::message_text;
use crate::{Peer, Scenario};

fn connected_network_change<'a>(
    event: &'a Event,
    id: &NetworkId,
) -> Option<&'a BouncerNetworkChanged> {
    event.as_bouncer_network_changed().filter(|changed| {
        &changed.id == id
            && changed
                .network
                .as_ref()
                .is_some_and(|network| network.state == Some(NetworkState::Connected))
    })
}

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
    loop {
        match events.next().await {
            Some(Event::Connected(_)) => break,
            Some(Event::Disconnected(disconnected)) => {
                panic!("bound client disconnected before reconnecting: {disconnected}")
            }
            Some(_) => {}
            None => panic!("bound client stopped before reconnecting"),
        }
    }

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

/// emits each bouncer change on one explicitly selected connection
#[scenario(
    requires(caps = ["soju.im/bouncer-networks"]),
    covers = ["security.bouncer.single-notification"]
)]
async fn bouncer_notifications_are_single_source(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile is not the managed soju bouncer");
    }
    let (mut control, id) = provisioned_control(&s, "notification-source").await?;
    let mut bound = s.connect_with(control.bouncer().network(&id)).await?;
    control.expect_none(Event::as_message).await?;
    bound.expect_none(Event::as_message).await?;

    let bouncer = s.server.bouncer().unwrap();
    let upstream = bouncer.upstream_addr();
    let config = |prefix: &str| {
        let name = papo_test_server::unique(prefix);
        NetworkConfig::new(upstream.ip().to_string())
            .port(upstream.port())
            .tls(false)
            .name(name.clone())
            .nick(name)
    };
    let added = control.bouncer().add(config("managed-change")).await?;
    let changed = control
        .wait_for(|event| connected_network_change(event, &added))
        .await?;
    assert!(changed.network.is_some());
    control
        .expect_none(|event| connected_network_change(event, &added))
        .await?;
    bound.expect_none(Event::as_bouncer_network_changed).await?;

    let notify = "soju.im/bouncer-networks-notify";
    let mut opted_in = s
        .connect_with(control.bouncer().network(&id).request_caps([notify]))
        .await?;
    opted_in.expect_none(Event::as_message).await?;
    let added = opted_in
        .bouncer()
        .add(config("bound-managed-change"))
        .await?;
    opted_in
        .wait_for(|event| connected_network_change(event, &added))
        .await?;
    opted_in
        .expect_none(|event| connected_network_change(event, &added))
        .await?;
    bound.expect_none(Event::as_bouncer_network_changed).await?;
    Ok(())
}
