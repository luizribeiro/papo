use libtest_mimic::{Completion, Failed};
use papo::{Account, Event};
use papo_conformance_macros::scenario;

use super::support::{EXPECT_TIMEOUT, close_pair, join, joined_pair};
use crate::{Scenario, ScenarioResult};

/// exposes the joining account and real name to every channel member
#[scenario(
    requires(caps = ["extended-join"]),
    covers = ["ircv3.extended-join"]
)]
async fn extended_join(context: Scenario) -> ScenarioResult {
    let mut bob = context.connect("bob").await?;
    let mut alice = context.connect_as_account("alice").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    let account = alice.state().account().unwrap().clone();
    bob.join(&channel).await.map_err(Failed::from)?;
    let joined = bob.wait_for(Event::as_joined).await?;
    assert_eq!(joined.channel.as_ref(), channel);
    assert_eq!(joined.nick, bob_nick);
    assert!(joined.is_self);

    alice.join(&channel).await.map_err(Failed::from)?;
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
    Ok(Completion::Completed)
}

/// records complete userhosts from the initial names reply
#[scenario(
    requires(caps = ["userhost-in-names"]),
    covers = ["base.names.basic", "ircv3.userhost-in-names"]
)]
async fn userhost_in_names(context: Scenario) -> ScenarioResult {
    let alice = context.connect("alice").await?;
    let bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    let bob_nick = bob.nick();
    alice.join(&channel).await.map_err(Failed::from)?;

    bob.join(&channel).await.map_err(Failed::from)?;

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
    Ok(Completion::Completed)
}

/// returns a complete identity from an extended WHO query
#[scenario(
    requires(isupport = ["WHOX"]),
    covers = ["base.who.whox-extended", "ircv3.whox"]
)]
async fn whox(context: Scenario) -> ScenarioResult {
    let alice = context
        .connect_with(context.configure("alice").realname("Alice Example"))
        .await?;
    let nick = alice.nick();

    let entries = alice.who(nick.as_ref()).await.map_err(Failed::from)?;
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
    Ok(Completion::Completed)
}

/// records every advertised membership prefix
#[scenario(requires(caps = ["multi-prefix"]))]
async fn multi_prefix(context: Scenario) -> ScenarioResult {
    let first_nick = context.unique();
    let second_nick = context.unique();
    let channel = format!("#{}", context.unique());
    let mut first = context.client(&first_nick).await?;
    join(&context, &mut first, &channel).await?;
    first
        .client
        .mode(&channel)
        .voice(&first_nick)
        .await
        .map_err(Failed::from)?;
    let casemapping = first.client.state().casemapping();
    context
        .expect(
            &mut first.events,
            |event| {
                matches!(event, Event::ModeChanged(changed) if casemapping.eq(&changed.target, &channel))
            },
            EXPECT_TIMEOUT,
        )
        .await?;
    let mut second = context.client(&second_nick).await?;
    join(&context, &mut second, &channel).await?;
    let prefixes = second
        .client
        .state()
        .channel(&channel)
        .and_then(|channel| channel.member(&first_nick))
        .map(|member| member.prefixes().to_vec());
    if prefixes.as_deref() != Some(&['@', '+']) {
        return Err(
            format!("NAMES did not preserve operator and voice prefixes: {prefixes:?}").into(),
        );
    }
    close_pair(first, second).await
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
