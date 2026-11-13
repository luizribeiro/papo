use std::{io, process::Command, time::Duration};

use papo::{
    Client, DisconnectCause, Endpoint, Error, Event, Feature, InvalidInput, NetworkId,
    NetworkState, Reconnect, Sasl, event::BouncerNetworkChanged,
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
    let login = bouncer.provision_network(&network_name, "papo bouncer control", None)?;
    let address = s.server.plaintext_addr();
    let control = s
        .connect_with(
            Client::builder(
                Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap(),
                &network_name,
            )
            .sasl(Sasl::plain(
                login.account_username(),
                papo::Secret::new(login.password()),
            ))
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
    assert_eq!(first.snapshot().bouncer_network(), Some(&id));
    assert_eq!(second.snapshot().bouncer_network(), Some(&id));
    first.wait_for(Event::as_channel_synced).await?;
    second.wait_for(Event::as_channel_synced).await?;

    let upstream = bouncer.upstream_addr();
    let mut sender = s
        .connect_with(
            Client::builder(
                Endpoint::plaintext(upstream.ip().to_string(), upstream.port()).unwrap(),
                papo_test_server::unique("origin-sender"),
            )
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
    assert_ne!(first_message.conversation(), second_message.conversation());
    assert_eq!(first_message.conversation(), channel.as_str());
    assert_eq!(second_message.conversation(), channel.as_str());

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
        match events.next().await.map(|(event, _)| event) {
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
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Disconnected(disconnected)) => break disconnected,
            Some(_) => {}
            None => panic!("bound client stopped before reporting the deletion"),
        }
    };
    assert!(reconnecting.is_reconnecting());
    let stopped = loop {
        match events.next().await.map(|(event, _)| event) {
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
        (format!("irc://{upstream}"), name)
    };
    let (endpoint, name) = config("managed-change");
    let added = control
        .bouncer()
        .add(endpoint)
        .name(&name)
        .nick(name)
        .await?;
    let changed = control
        .wait_for(|event| connected_network_change(event, &added))
        .await?;
    assert!(changed.network.is_some());
    control
        .expect_none(|event| connected_network_change(event, &added))
        .await?;
    bound.expect_none(Event::as_bouncer_network_changed).await?;

    let mut opted_in = s
        .connect_with(
            control
                .bouncer()
                .network(&id)
                .enable(papo::Feature::Bouncer),
        )
        .await?;
    opted_in.expect_none(Event::as_message).await?;
    let (endpoint, name) = config("bound-managed-change");
    let added = opted_in
        .bouncer()
        .add(endpoint)
        .name(&name)
        .nick(name)
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

fn set_network_enabled(
    bouncer: &papo_test_server::BouncerInfo,
    username: &str,
    network: &str,
    enabled: bool,
) -> io::Result<()> {
    let status = Command::new(std::env::var("SOJUCTL_BIN").map_err(io::Error::other)?)
        .args(["-config"])
        .arg(bouncer.config_path())
        .args([
            "user",
            "run",
            username,
            "network",
            "update",
            network,
            "-enabled",
            if enabled { "true" } else { "false" },
        ])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "sojuctl could not update the upstream network: {status}"
        )))
    }
}

/// rejects a continuity pin and live target after a bouncer upstream session ends
#[scenario(
    requires(caps = ["soju.im/bouncer-networks"]),
    covers = ["security.bouncer.upstream-drop-identity"]
)]
async fn bouncer_upstream_drop_ends_peer_identity(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile is not the managed soju bouncer");
    }
    let bouncer = s.server.bouncer().unwrap();
    let network = papo_test_server::unique("identity-drop");
    let login = bouncer.provision_network(&network, "papo identity observer", None)?;
    let bouncer_address = s.server.plaintext_addr();
    let control = s
        .connect_with(
            Client::builder(
                Endpoint::plaintext(bouncer_address.ip().to_string(), bouncer_address.port())
                    .unwrap(),
                &network,
            )
            .sasl(Sasl::plain(
                login.account_username(),
                papo::Secret::new(login.password()),
            ))
            .allow_insecure_auth(true)
            .enable(Feature::Bouncer),
        )
        .await?;
    let id = control
        .bouncer()
        .networks()
        .await?
        .into_iter()
        .find(|candidate| candidate.name.as_deref() == Some(network.as_str()))
        .unwrap()
        .id;
    let channel = s.channel_name();
    let mut observer = s
        .connect_with(
            control
                .bouncer()
                .network(&id)
                .enable(Feature::Bouncer)
                .autojoin([&channel]),
        )
        .await?;
    observer.wait_for(Event::as_channel_synced).await?;

    let upstream = bouncer.upstream_addr();
    let upstream_endpoint =
        Endpoint::plaintext(upstream.ip().to_string(), upstream.port()).unwrap();
    let alice_nick = papo_test_server::unique("alice-identity");
    let alice = s
        .connect_with(Client::builder(upstream_endpoint.clone(), &alice_nick))
        .await?;
    alice.join(&channel).await?;
    observer
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == alice_nick.as_str())
        })
        .await?;
    let alice_handle = observer.query(&alice_nick)?;
    let stored = alice_handle.key();
    assert!(!stored.is_verified());

    set_network_enabled(bouncer, login.account_username(), &network, false)?;
    observer
        .wait_for(|event| {
            event.as_bouncer_network_changed().filter(|changed| {
                changed.id == id
                    && changed
                        .network
                        .as_ref()
                        .is_some_and(|network| network.state == Some(NetworkState::Disconnected))
            })
        })
        .await?;
    assert!(matches!(
        observer
            .privmsg(&alice_handle, "must not reach a replacement")
            .await,
        Err(Error::TargetGone)
    ));

    alice.quit("identity handoff during outage").await?;
    let stranger = s
        .connect_with(Client::builder(upstream_endpoint, &alice_nick))
        .await?;
    stranger.join(&channel).await?;
    set_network_enabled(bouncer, login.account_username(), &network, true)?;
    observer.wait_for(Event::as_channel_synced).await?;

    let Error::NotSamePerson(mismatch) = observer.open(&stored).await.unwrap_err() else {
        panic!("the old continuity pin unexpectedly authorized the replacement");
    };
    assert_eq!(mismatch.reason(), papo::PeerKeyMismatch::CannotVerify);
    assert!(matches!(
        stored.check(mismatch.current_key()),
        papo::PeerKeyCheck::Mismatch(papo::PeerKeyMismatch::CannotVerify)
    ));
    Ok(())
}
