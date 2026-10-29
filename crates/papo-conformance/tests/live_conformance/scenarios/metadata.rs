use papo::{Event, MetadataKey, Visibility};
use papo_conformance_macros::scenario;

use crate::{Scenario, scenarios::support::subject_name};

/// sets, gets, and removes client metadata
#[scenario(
    requires(caps = ["batch", "draft/metadata-2"]),
    covers = ["draft.metadata.set-get-self"]
)]
async fn draft_metadata_2(s: Scenario) -> papo_testing::Result<()> {
    let key = MetadataKey::new("example.org/papo-conformance");
    let value = "fixture metadata value";
    let mut alice = s.connect("alice").await?;
    let nick = alice.state().nick().clone();

    alice.metadata().set(key.clone(), value).await?;
    let changed = alice.wait_for(Event::as_metadata_changed).await?;
    assert_eq!(subject_name(&changed.subject), nick.as_ref());
    assert_eq!(changed.key, key);
    assert_eq!(
        changed.value.as_ref().map(|value| value.value.as_str()),
        Some(value)
    );
    assert_eq!(
        changed.value.as_ref().unwrap().visibility,
        Visibility::Public
    );

    let values = alice.metadata().get(nick.clone(), [key.clone()]).await?;
    let stored = values
        .get(key.as_ref())
        .expect("the requested metadata key is returned")
        .as_ref()
        .expect("the requested metadata key is set");
    assert_eq!(stored.value, value);
    assert_eq!(stored.visibility, Visibility::Public);
    assert_eq!(
        alice.state().metadata(&nick).unwrap().get(key.as_ref()),
        Some(stored)
    );

    alice.metadata().remove(key.clone()).await?;
    let removed = alice
        .wait_for(|event| {
            event
                .as_metadata_changed()
                .filter(|changed| changed.value.is_none())
        })
        .await?;
    assert_eq!(subject_name(&removed.subject), nick.as_ref());
    assert_eq!(removed.key, key);
    assert_eq!(removed.value, None);
    assert!(
        alice
            .state()
            .metadata(&nick)
            .is_none_or(|values| values.get(key.as_ref()).is_none())
    );
    alice.expect_none(Event::as_metadata_changed).await?;
    Ok(())
}

/// notifies subscribed channel peers about client metadata
#[scenario(
    requires(caps = ["batch", "draft/metadata-2"]),
    covers = ["draft.metadata.sub-notifications"]
)]
async fn metadata_subscriptions(s: Scenario) -> papo_testing::Result<()> {
    let key = MetadataKey::new("example.org/papo-presence");
    let channel = s.channel_name();
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    alice.join(channel.clone()).await?;
    bob.join(channel).await?;
    bob.metadata().subscribe([key.clone()]).await?;
    assert!(
        bob.state()
            .metadata_subscriptions()
            .any(|subscribed| subscribed == &key)
    );

    let alice_nick = alice.state().nick().clone();
    alice.metadata().set(key.clone(), "available").await?;
    let alice_change = alice
        .wait_for(|event| {
            event
                .as_metadata_changed()
                .filter(|changed| changed.key == key)
        })
        .await?;
    let bob_change = bob
        .wait_for(|event| {
            event.as_metadata_changed().filter(|changed| {
                subject_name(&changed.subject) == alice_nick.as_ref() && changed.key == key
            })
        })
        .await?;
    assert_eq!(subject_name(&alice_change.subject), alice_nick.as_ref());
    assert_eq!(alice_change.key, bob_change.key);
    assert_eq!(alice_change.value, bob_change.value);
    assert_eq!(
        bob_change.value.as_ref().map(|value| value.value.as_str()),
        Some("available")
    );
    assert_eq!(
        bob_change.value.as_ref().unwrap().visibility,
        Visibility::Public
    );
    let bob_state = bob.state();
    let cached = bob_state
        .metadata(alice_nick.as_ref())
        .expect("subscriber caches the visible metadata");
    assert_eq!(cached.get(key.as_ref()).unwrap().value, "available");

    bob.metadata().unsubscribe([key.clone()]).await?;
    assert!(bob.state().metadata_subscriptions().next().is_none());
    alice.metadata().set(key, "busy").await?;
    bob.expect_none(Event::as_metadata_changed).await?;
    let current = bob.state();
    assert!(
        current
            .metadata(alice_nick.as_ref())
            .unwrap()
            .iter()
            .eq(cached.iter())
    );
    Ok(())
}

/// prevents cached metadata from crossing a nickname handoff
#[scenario(
    requires(caps = ["batch", "draft/metadata-2"]),
    covers = ["security.st-1.metadata-nick-handoff"]
)]
async fn metadata_does_not_cross_nick_handoffs(s: Scenario) -> papo_testing::Result<()> {
    let key = MetadataKey::new("example.org/papo-identity");
    let channel = s.channel_name();
    let mut observer = s.connect("observer").await?;
    let alice = s.connect("alice").await?;
    let mallory = s.connect("mallory").await?;
    observer.join(&channel).await?;
    alice.join(&channel).await?;
    mallory.join(&channel).await?;
    observer.metadata().subscribe([key.clone()]).await?;

    let alice_nick = alice.nick();
    let mallory_nick = mallory.nick();
    alice.metadata().set(key.clone(), "Alice Admin").await?;
    observer
        .wait_for(|event| {
            event.as_metadata_changed().filter(|changed| {
                subject_name(&changed.subject) == alice_nick.as_ref() && changed.key == key
            })
        })
        .await?;
    assert!(observer.state().metadata(alice_nick.as_ref()).is_some());

    alice.quit("handoff").await?;
    observer
        .wait_for(|event| {
            event
                .as_quit()
                .filter(|quit| quit.actor.nick() == alice_nick.as_ref())
        })
        .await?;
    assert!(observer.state().metadata(alice_nick.as_ref()).is_none());

    mallory.set_nick(alice_nick.as_ref()).await?;
    observer
        .wait_for(|event| {
            event.as_nick_changed().filter(|changed| {
                changed.actor.nick() == mallory_nick.as_ref() && changed.new == alice_nick.as_ref()
            })
        })
        .await?;
    assert!(observer.state().metadata(alice_nick.as_ref()).is_none());
    Ok(())
}

/// synchronizes channel metadata to current and later members
#[scenario(
    requires(caps = ["batch", "draft/metadata-2"]),
    covers = ["draft.metadata.channel-metadata"]
)]
async fn channel_metadata(s: Scenario) -> papo_testing::Result<()> {
    let key = MetadataKey::new("example.org/papo-topic-category");
    let value = "systems programming";
    let channel = s.channel_name();
    let alice = s.connect("alice").await?;
    let mut bob = s
        .connect_with(s.configure("bob")?.metadata_subscriptions([key.clone()]))
        .await?;
    alice.join(channel.clone()).await?;
    bob.join(channel.clone()).await?;

    alice
        .metadata()
        .set_on(channel.clone(), key.clone(), value)
        .await?;
    let bob_change = bob
        .wait_for(|event| {
            event
                .as_metadata_changed()
                .filter(|changed| subject_name(&changed.subject) == channel && changed.key == key)
        })
        .await?;
    assert_eq!(
        bob_change.value.as_ref().map(|value| value.value.as_str()),
        Some(value)
    );
    assert_eq!(
        bob_change.value.as_ref().unwrap().visibility,
        Visibility::Public
    );

    let mut charlie = s
        .connect_with(
            s.configure("charlie")?
                .metadata_subscriptions([key.clone()]),
        )
        .await?;
    charlie.join(channel.clone()).await?;
    let charlie_change = charlie
        .wait_for(|event| {
            event
                .as_metadata_changed()
                .filter(|changed| subject_name(&changed.subject) == channel && changed.key == key)
        })
        .await?;
    assert_eq!(subject_name(&charlie_change.subject), channel);
    assert_eq!(charlie_change.key, bob_change.key);
    assert_eq!(charlie_change.value, bob_change.value);
    for peer in [&alice, &bob, &charlie] {
        let state = peer.state();
        let values = state
            .metadata(&channel)
            .expect("channel metadata is synchronized");
        assert_eq!(values.get(key.as_ref()).unwrap().value, value);
        assert_eq!(
            values.get(key.as_ref()).unwrap().visibility,
            Visibility::Public
        );
    }
    Ok(())
}

/// rejects metadata writes to another client
#[scenario(
    requires(caps = ["batch", "draft/metadata-2"]),
    covers = ["draft.metadata.errors"]
)]
async fn metadata_error(s: Scenario) -> papo_testing::Result<()> {
    let key = MetadataKey::new("example.org/papo-protected");
    let channel = s.channel_name();
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    alice.join(channel.clone()).await?;
    bob.join(channel).await?;

    let error = bob
        .metadata()
        .set_on(alice.state().nick().as_ref(), key, "x")
        .await
        .expect_err("a client cannot write another client's metadata");
    let papo::Error::Server(error) = error else {
        panic!("metadata refusal retains a server error: {error:?}")
    };
    assert!(error.command().is_some_and(|command| command == "METADATA"));
    assert!(matches!(
        error.code().as_str(),
        "KEY_NO_PERMISSION" | "KEY_INVALID"
    ));
    assert!(alice.state().metadata(alice.state().nick()).is_none());
    alice.expect_none(Event::as_metadata_changed).await?;
    bob.expect_none(Event::as_metadata_changed).await?;
    Ok(())
}

/// rejects a channel rename that collides with another channel
#[scenario(
    requires(caps = ["draft/channel-rename"]),
    covers = ["draft.channel-rename.errors"]
)]
async fn channel_rename_error(s: Scenario) -> papo_testing::Result<()> {
    let old = s.channel_name();
    let occupied = s.channel_name();
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    alice.join(old.clone()).await?;
    bob.join(occupied.clone()).await?;

    let error = alice
        .rename_channel(old.clone(), occupied.clone(), "collision")
        .await
        .expect_err("a rename cannot replace an existing channel");
    let papo::Error::Server(error) = error else {
        panic!("rename refusal retains a server error: {error:?}")
    };
    assert!(error.command().is_some_and(|command| command == "RENAME"));
    assert_eq!(error.code(), "CHANNEL_NAME_IN_USE");
    assert!(alice.state().channel(&old).is_some());
    assert!(alice.state().channel(&occupied).is_none());
    assert!(bob.state().channel(&occupied).is_some());
    alice.expect_none(Event::as_channel_renamed).await?;
    bob.expect_none(Event::as_channel_renamed).await?;
    Ok(())
}

/// renames a channel for every joined member
#[scenario(
    requires(caps = ["draft/channel-rename"]),
    covers = ["draft.channel-rename.rename"]
)]
async fn draft_channel_rename(s: Scenario) -> papo_testing::Result<()> {
    let channel = s.channel_name();
    let renamed = format!("{channel}-renamed");
    let reason = "the project moved";
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let alice_nick = alice.nick();
    alice.join(channel.clone()).await?;
    bob.join(channel.clone()).await?;

    alice
        .rename_channel(channel.clone(), renamed.clone(), reason)
        .await?;
    let alice_rename = alice.wait_for(Event::as_channel_renamed).await?;
    let bob_rename = bob.wait_for(Event::as_channel_renamed).await?;
    for event in [&alice_rename, &bob_rename] {
        assert_eq!(event.old.as_ref(), channel);
        assert_eq!(event.new.as_ref(), renamed);
        assert_eq!(crate::source_nick(&event.actor), Some(alice_nick.as_ref()));
        assert_eq!(event.reason.as_deref(), Some(reason));
    }
    for peer in [&alice, &bob] {
        let snapshot = peer.state();
        assert!(snapshot.channel(&channel).is_none());
        let state = snapshot
            .channel(&renamed)
            .expect("the renamed channel remains joined");
        assert!(state.is_synced());
    }
    alice.expect_none(Event::as_channel_renamed).await?;
    bob.expect_none(Event::as_channel_renamed).await?;
    Ok(())
}

/// delivers a relayed message with attribution to the submitting client
#[scenario(
    requires(caps = ["draft/relaymsg", "message-tags"]),
    covers = ["ergo.relaymsg"]
)]
async fn relay_message(s: Scenario) -> papo_testing::Result<()> {
    let alice = s
        .connect_with(s.configure("alice")?.enable(papo::Feature::Relay))
        .await?;
    let mut bob = s
        .connect_with(s.configure("bob")?.enable(papo::Feature::Relay))
        .await?;
    let channel = s.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let text = "Hello from the bridged network";
    let mapping = alice.state().casemapping();
    alice
        .relay(
            mapping.channel(&channel),
            mapping.nick("matrix/alice"),
            text,
        )
        .await?;
    let message = bob
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(message.relayed_by.as_deref(), Some(alice_nick.as_ref()));
    assert_eq!(
        crate::scenarios::support::conversation_name(message.conversation()),
        Some(channel.as_str())
    );
    assert_eq!(crate::source_nick(&message.actor), Some("matrix/alice"));
    Ok(())
}
