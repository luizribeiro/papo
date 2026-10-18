use papo::{Event, event::Visibility};
use papo_conformance_macros::scenario;

use crate::Scenario;

/// sets, gets, and removes client metadata
#[scenario(
    requires(caps = ["batch", "draft/metadata-2"]),
    covers = ["draft.metadata.set-get-self"]
)]
async fn draft_metadata_2(s: Scenario) -> papo_testing::Result<()> {
    let key = "example.org/papo-conformance";
    let value = "fixture metadata value";
    let mut alice = s
        .connect_with(
            s.configure("alice")
                .request_caps(["batch", "draft/metadata-2"]),
        )
        .await?;
    let nick = alice.state().nick().clone();

    alice.metadata().set(key, value).await?;
    let changed = alice.wait_for(Event::as_metadata_changed).await?;
    assert_eq!(changed.target.as_ref(), nick.as_ref());
    assert_eq!(changed.key.as_ref(), key);
    assert_eq!(changed.value.as_deref(), Some(value));
    assert_eq!(changed.visibility, Visibility::Public);

    let values = alice.metadata().get(nick.clone(), [key]).await?;
    let stored = values
        .get(key)
        .expect("the requested metadata key is cached");
    assert_eq!(stored.value, value);
    assert_eq!(stored.visibility, "*");
    assert_eq!(alice.state().metadata(&nick), Some(values));

    alice.metadata().remove(key).await?;
    let removed = alice
        .wait_for(|event| {
            event
                .as_metadata_changed()
                .filter(|changed| changed.value.is_none())
        })
        .await?;
    assert_eq!(removed.target.as_ref(), nick.as_ref());
    assert_eq!(removed.key.as_ref(), key);
    assert_eq!(removed.value, None);
    assert_eq!(removed.visibility, Visibility::Public);
    assert!(
        alice
            .state()
            .metadata(&nick)
            .is_none_or(|values| !values.contains_key(key))
    );
    alice.expect_none(Event::as_metadata_changed).await?;
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
    let mut alice = s
        .connect_with(s.configure("alice").request_caps(["draft/channel-rename"]))
        .await?;
    let mut bob = s
        .connect_with(s.configure("bob").request_caps(["draft/channel-rename"]))
        .await?;
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
        assert_eq!(event.reason, reason);
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
