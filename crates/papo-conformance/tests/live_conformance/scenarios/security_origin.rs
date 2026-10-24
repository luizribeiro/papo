use papo::{Client, Endpoint, Error, Event, Feature, InvalidInput};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::message_text;

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
    let network_name = papo_test_server::unique("origin-network");
    let login = bouncer.provision_network(&network_name)?;
    let address = s.server.plaintext_addr();
    let control = s
        .connect_with(
            Client::builder(Endpoint::plaintext(address.ip().to_string(), address.port()).unwrap())
                .nick(&network_name)
                .username(login.username())
                .password(login.password())
                .allow_insecure_auth(true)
                .enable(Feature::Bouncer),
        )
        .await?;
    let id = control
        .bouncer()
        .networks()
        .await?
        .into_iter()
        .find(|network| network.name.as_deref() == Some(network_name.as_str()))
        .expect("the provisioned network is listed")
        .id;
    let channel = s.channel_name();
    let mut first = s
        .connect_with(control.bouncer().network(&id).autojoin([&channel]))
        .await?;
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
