use libtest_mimic::Failed;
use papo::{Account, Event, ModeKind};
use papo_conformance_macros::scenario;

use super::support::{EXPECT_TIMEOUT, close_pair, joined_pair};
use crate::{Scenario, ScenarioResult};

/// exposes the joining account and real name to every channel member
#[scenario(
    requires(caps = ["extended-join"]),
    covers = ["ircv3.extended-join"]
)]
async fn extended_join(context: Scenario) -> papo_testing::Result<()> {
    let mut bob = context.connect("bob").await?;
    let mut alice = context.connect_as_account("alice").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let account = alice.state().account().unwrap().clone();
    bob.join(&channel).await?;
    let joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, bob_nick);
    assert!(joined.is_self);

    alice.join(&channel).await?;
    let joined = alice.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, alice_nick);
    assert!(joined.is_self);
    let joined = bob.wait_for(Event::as_joined).await?;

    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, alice_nick);
    assert!(!joined.is_self);
    let bob_state = bob.state();
    let observed = bob_state.user(&alice_nick).unwrap();
    assert_eq!(observed.realname(), Some("papo test peer"));
    assert!(matches!(observed.account(), Account::LoggedIn(name) if name == &account));
    assert!(
        bob_state
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_some()
    );
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_some()
    );
    bob.expect_none(Event::as_joined).await?;
    Ok(())
}

/// records complete userhosts from the initial names reply
#[scenario(
    requires(caps = ["userhost-in-names"]),
    covers = ["base.names.basic", "ircv3.userhost-in-names"]
)]
async fn userhost_in_names(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;

    bob.join(&channel).await?;

    let bob_state = bob.state();
    let observed = bob_state.user(&alice_nick).unwrap();
    assert!(observed.username().is_some_and(|user| !user.is_empty()));
    assert!(observed.host().is_some_and(|host| !host.is_empty()));
    assert!(
        bob_state
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_some()
    );
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_some()
    );
    Ok(())
}

/// returns a complete identity from an extended WHO query
#[scenario(
    requires(isupport = ["WHOX"]),
    covers = ["base.who.whox-extended", "ircv3.whox"]
)]
async fn whox(context: Scenario) -> papo_testing::Result<()> {
    let alice = context
        .connect_with(context.configure("alice").realname("Alice Example"))
        .await?;
    let nick = alice.nick();

    let entries = alice.who(nick.as_ref()).await?;
    let entry = entries
        .iter()
        .find(|entry| alice.state().casemapping().eq(&entry.nick, &nick))
        .expect("WHO must include its exact target");

    assert_eq!(entry.nick, nick);
    assert!(!entry.user.is_empty());
    assert!(!entry.host.is_empty());
    assert_eq!(entry.realname, "Alice Example");
    assert!(!entry.away);
    assert!(!entry.is_oper);
    Ok(())
}

/// preserves every membership prefix in the joined member list
#[scenario(
    requires(caps = ["multi-prefix"]),
    covers = ["base.isupport.prefix", "ircv3.multi-prefix"]
)]
async fn multi_prefix(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;

    alice.mode(&channel).voice(alice_nick.clone()).await?;
    let changed = alice.wait_for(Event::as_mode_changed).await?;

    assert_eq!(changed.target.as_ref(), channel);
    assert_eq!(changed.by.nick(), Some(alice_nick.as_ref()));
    assert_eq!(changed.changes.len(), 1);
    assert!(changed.changes[0].set);
    assert_eq!(changed.changes[0].mode, 'v');
    let voice_rank = alice
        .state()
        .isupport()
        .prefix()
        .iter()
        .find(|prefix| prefix.mode == 'v')
        .unwrap()
        .rank;
    assert_eq!(changed.changes[0].kind, ModeKind::Prefix(voice_rank));
    assert_eq!(changed.changes[0].arg.as_deref(), Some(alice_nick.as_ref()));
    bob.join(&channel).await?;
    let prefixes = bob
        .state()
        .channel(&channel)
        .and_then(|channel| channel.member(&alice_nick))
        .map(|member| member.prefixes().to_vec());
    assert_eq!(prefixes.as_deref(), Some(&['@', '+'][..]));
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_some()
    );
    bob.expect_none(Event::as_mode_changed).await?;
    Ok(())
}

/// notifies channel members about invitations
#[scenario(requires(caps = ["invite-notify"]))]
async fn invite_notify(context: Scenario) -> ScenarioResult {
    let (inviter, mut observer, channel) = joined_pair(&context).await?;
    let observer_nick = observer.welcome.nick.clone();
    if !observer.client.state().has_cap("invite-notify") {
        return Err("invite-notify observer did not negotiate the capability".into());
    }
    inviter
        .client
        .mode(&channel)
        .op(&observer_nick)
        .await
        .map_err(Failed::from)?;
    let casemapping = observer.client.state().casemapping();
    context
        .expect(
            &mut observer.events,
            |event| {
                matches!(event, Event::ModeChanged(changed) if casemapping.eq(&changed.target, &channel))
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    let observer_is_op = observer
        .client
        .state()
        .channel(&channel)
        .and_then(|channel| channel.member(&observer_nick))
        .is_some_and(papo_core::Member::is_op);
    if !observer_is_op {
        return Err("invite-notify observer is not a channel operator".into());
    }
    let target_nick = context.unique();
    let mut target = context.client(&target_nick).await?;
    inviter
        .client
        .invite(&channel, &target_nick)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut observer.events,
            |event| {
                matches!(event, Event::Invited(invite_event)
                    if invite_event.nick.as_ref() == target_nick
                        && invite_event.channel.as_ref() == channel
                        && !invite_event.is_self)
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    context
        .expect(
            &mut target.events,
            |event| {
                matches!(event, Event::Invited(invite_event)
                    if invite_event.nick.as_ref() == target_nick
                        && invite_event.channel.as_ref() == channel
                        && invite_event.is_self)
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    observer.close().await?;
    close_pair(inviter, target).await
}
