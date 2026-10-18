use papo::{Account, Event, ModeKind};
use papo_conformance_macros::scenario;

use crate::Scenario;

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

/// delivers an invitation to its target and every capable channel observer
#[scenario(
    requires(caps = ["invite-notify"]),
    covers = ["base.invite.target-delivery", "ircv3.invite-notify"]
)]
async fn invite_notify(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let mut carol = context.connect("carol").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.mode(&channel).op(bob_nick.clone()).await?;
    let promoted = bob.wait_for(Event::as_mode_changed).await?;

    assert_eq!(promoted.target.as_ref(), channel);
    assert_eq!(promoted.by.nick(), Some(alice_nick.as_ref()));
    assert_eq!(promoted.changes.len(), 1);
    assert!(promoted.changes[0].set);
    assert_eq!(promoted.changes[0].mode, 'o');
    assert_eq!(promoted.changes[0].arg.as_deref(), Some(bob_nick.as_ref()));
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .unwrap()
            .is_op()
    );

    alice.invite(&channel, carol_nick.clone()).await?;
    let target = carol.wait_for(Event::as_invited).await?;
    let observer = bob.wait_for(Event::as_invited).await?;

    assert_eq!(target.channel.as_ref(), channel);
    assert_eq!(target.nick.as_ref(), carol_nick.as_ref());
    assert_eq!(target.by.nick(), Some(alice_nick.as_ref()));
    assert!(target.is_self);
    assert_eq!(observer.channel.as_ref(), channel);
    assert_eq!(observer.nick.as_ref(), carol_nick.as_ref());
    assert_eq!(observer.by.nick(), Some(alice_nick.as_ref()));
    assert!(!observer.is_self);
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .is_none()
    );
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&carol_nick)
            .is_none()
    );
    carol.expect_none(Event::as_invited).await?;
    Ok(())
}

/// removes a departing member from both peers with the same reason
#[scenario(covers = ["base.part.basic"])]
async fn part(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let reason = "leaving for the specification review";
    alice.part(&channel, reason).await?;
    let own = alice.wait_for(Event::as_parted).await?;
    let observed = bob.wait_for(Event::as_parted).await?;

    for departed in [&own, &observed] {
        assert_eq!(departed.channel.as_ref(), channel);
        assert_eq!(departed.nick, alice_nick);
        assert_eq!(departed.reason.as_deref(), Some(reason));
    }
    assert!(own.is_self);
    assert!(!observed.is_self);
    assert!(alice.state().channel(&channel).is_none());
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&alice_nick)
            .is_none()
    );
    alice.expect_none(Event::as_parted).await?;
    bob.expect_none(Event::as_parted).await?;
    Ok(())
}
