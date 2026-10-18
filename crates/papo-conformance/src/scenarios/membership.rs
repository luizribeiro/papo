use papo::{Account, Error, Event, ModeKind, Refusal};
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

/// removes a kicked member from the channel state of the target and operator
#[scenario(covers = ["base.kick.basic"])]
async fn kick(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let reason = "refresh channel membership";
    alice.kick(&channel, bob_nick.clone(), reason).await?;
    let observed = alice.wait_for(Event::as_kicked).await?;
    let own = bob.wait_for(Event::as_kicked).await?;

    for kicked in [&observed, &own] {
        assert_eq!(kicked.channel.as_ref(), channel);
        assert_eq!(kicked.nick, bob_nick);
        assert_eq!(kicked.by.nick(), Some(alice_nick.as_ref()));
        assert_eq!(kicked.reason.as_deref(), Some(reason));
    }
    assert!(!observed.is_self);
    assert!(own.is_self);
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&bob_nick)
            .is_none()
    );
    assert!(bob.state().channel(&channel).is_none());
    alice.expect_none(Event::as_kicked).await?;
    bob.expect_none(Event::as_kicked).await?;
    Ok(())
}

/// propagates a nickname change and rejects a later claim of the same nickname
#[scenario(covers = ["base.nick.change", "base.nick.collision-in-use"])]
async fn nick_change_and_collision(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let old_nick = alice.nick();
    let bob_nick = bob.nick();
    let new_nick = format!("r{old_nick}");
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.set_nick(new_nick.clone()).await?;
    let own = alice.wait_for(Event::as_nick_changed).await?;
    let observed = bob.wait_for(Event::as_nick_changed).await?;

    for changed in [&own, &observed] {
        assert_eq!(changed.old, old_nick);
        assert_eq!(changed.new.as_ref(), new_nick);
    }
    assert!(own.is_self);
    assert!(!observed.is_self);
    assert_eq!(alice.state().nick(), new_nick.as_str());
    assert!(
        alice
            .state()
            .channel(&channel)
            .unwrap()
            .member(&new_nick)
            .is_some()
    );
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&new_nick)
            .is_some()
    );
    assert!(
        bob.state()
            .channel(&channel)
            .unwrap()
            .member(&old_nick)
            .is_none()
    );

    let Error::Server(refusal) = bob.set_nick(new_nick.clone()).await.unwrap_err() else {
        panic!("nickname collision did not return a server refusal");
    };
    assert_eq!(refusal.kind(), Refusal::NicknameInUse);
    assert_eq!(refusal.code(), "433");
    assert_eq!(bob.state().nick(), &bob_nick);
    bob.expect_none(Event::as_nick_changed).await?;
    Ok(())
}

/// synchronizes topic metadata and propagates setting and clearing the topic
#[scenario(covers = [
    "base.topic.get",
    "base.topic.set",
    "base.topic.clear",
    "base.topic.whotime"
])]
async fn topic(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let mut carol = context.connect("carol").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let text = "Reviewing IRC topic synchronization";
    alice.set_topic(&channel, text).await?;
    let own = alice.wait_for(Event::as_topic_changed).await?;
    let observed = bob.wait_for(Event::as_topic_changed).await?;

    for changed in [&own, &observed] {
        assert_eq!(changed.channel.as_ref(), channel);
        assert_eq!(changed.topic.as_deref(), Some(text));
        assert_eq!(changed.by.nick(), Some(alice_nick.as_ref()));
    }
    for peer in [&alice, &bob] {
        assert_eq!(peer.state().channel(&channel).unwrap().topic(), Some(text));
    }

    carol.join(&channel).await?;
    let state = carol.state();
    let synchronized = state.channel(&channel).unwrap();
    assert_eq!(synchronized.topic(), Some(text));
    assert_eq!(
        synchronized
            .topic_set_by()
            .and_then(|source| source.split('!').next()),
        Some(alice_nick.as_ref())
    );
    assert!(synchronized.topic_set_at().is_some());

    alice.clear_topic(&channel).await?;
    for peer in [&mut alice, &mut bob, &mut carol] {
        let changed = peer.wait_for(Event::as_topic_changed).await?;
        assert_eq!(changed.channel.as_ref(), channel);
        assert_eq!(changed.topic, None);
        assert_eq!(changed.by.nick(), Some(alice_nick.as_ref()));
        assert_eq!(peer.state().channel(&channel).unwrap().topic(), None);
        peer.expect_none(Event::as_topic_changed).await?;
    }
    Ok(())
}

/// enforces channel admission and messaging modes with typed refusals
#[scenario(covers = [
    "base.join.key-single",
    "base.join.err-bad-key",
    "base.join.err-banned",
    "base.join.err-invite-only",
    "base.join.err-channel-full",
    "base.mode.channel.ban",
    "base.mode.channel.key",
    "base.mode.channel.limit",
    "base.mode.channel.moderated",
    "base.mode.channel.secret"
])]
async fn channel_modes_and_join_refusals(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let mut carol = context.connect("carol").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let carol_nick = carol.nick();
    let key = "correct-horse-battery-staple";
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    alice.mode(&channel).add_with('k', key).add('s').await?;
    let modes = bob.wait_for(Event::as_mode_changed).await?;
    assert_eq!(modes.target.as_ref(), channel);
    assert_eq!(modes.by.nick(), Some(alice_nick.as_ref()));
    assert_eq!(
        modes
            .changes
            .iter()
            .map(|change| (change.set, change.mode, change.arg.as_deref()))
            .collect::<Vec<_>>(),
        [(true, 'k', Some(key)), (true, 's', None)]
    );
    for peer in [&alice, &bob] {
        let state = peer.state();
        let channel_state = state.channel(&channel).unwrap();
        assert_eq!(channel_state.mode('k'), Some(Some(key)));
        assert_eq!(channel_state.mode('s'), Some(None));
    }

    assert_refusal(
        carol.join(&channel).key("wrong key").await.unwrap_err(),
        Refusal::BadChannelKey,
        "475",
    );
    alice.mode(&channel).add_with('l', "2").await?;
    bob.wait_for(Event::as_mode_changed).await?;
    assert_refusal(
        carol.join(&channel).key(key).await.unwrap_err(),
        Refusal::ChannelIsFull,
        "471",
    );

    alice.mode(&channel).remove('l').await?;
    bob.wait_for(Event::as_mode_changed).await?;
    let mask = format!("{carol_nick}!*@*");
    alice.mode(&channel).ban(mask.clone()).await?;
    bob.wait_for(Event::as_mode_changed).await?;
    assert_refusal(
        carol.join(&channel).key(key).await.unwrap_err(),
        Refusal::BannedFromChannel,
        "474",
    );

    alice.mode(&channel).unban(mask).add('i').await?;
    bob.wait_for(Event::as_mode_changed).await?;
    assert_refusal(
        carol.join(&channel).key(key).await.unwrap_err(),
        Refusal::InviteOnlyChannel,
        "473",
    );
    alice.invite(&channel, carol_nick.clone()).await?;
    let invited = carol.wait_for(Event::as_invited).await?;
    assert_eq!(invited.nick, carol_nick);
    assert_eq!(invited.channel.as_ref(), channel);
    carol.join(&channel).key(key).await?;

    alice.mode(&channel).add('m').await?;
    bob.wait_for(Event::as_mode_changed).await?;
    assert_refusal(
        bob.privmsg(&channel, "blocked while unvoiced")
            .await
            .unwrap_err(),
        Refusal::CannotSendToChannel,
        "404",
    );
    alice.mode(&channel).voice(bob_nick).await?;
    bob.wait_for(Event::as_mode_changed).await?;
    bob.privmsg(&channel, "allowed after voice").await?;
    let delivered = carol.wait_for(Event::as_message).await?;
    assert_eq!(delivered.text(), "allowed after voice");
    Ok(())
}

fn assert_refusal(error: Error, expected: Refusal, code: &str) {
    let Error::Server(error) = error else {
        panic!("operation did not return a server refusal");
    };
    assert_eq!(error.kind(), expected);
    assert_eq!(error.code(), code);
}
