use libtest_mimic::Failed;
use papo::{Account, Away, Event, Origin, Source};
use papo_conformance_macros::scenario;

use super::support::{EXPECT_TIMEOUT, close_pair};
use crate::{Scenario, ScenarioResult};

/// delivers an authenticated sender account
#[scenario(
    requires(caps = ["account-tag", "account-notify"]),
    covers = ["ircv3.account-tag.channel"]
)]
async fn account_notify(context: Scenario) -> papo_testing::Result<()> {
    let alice = context.connect_as_account("alice").await?;
    let mut bob = context.connect("bob").await?;
    let account = alice.state().account().unwrap().clone();
    let alice_nick = alice.nick();
    let channel = context.channel_name();
    alice.join(&channel).await?;
    bob.join(&channel).await?;

    let text = "authenticated hello from alice";
    let sent = alice.privmsg(&channel, text).await?;
    let message = bob.wait_for(Event::as_message).await?;

    assert_eq!(message.text(), text);
    assert_eq!(
        message.sender.as_ref().and_then(Source::nick),
        Some(alice_nick.as_ref())
    );
    assert_eq!(message.target(), channel);
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(message.server_time, sent.time);
    assert_eq!(message.account(), Some(account.as_ref()));
    assert_eq!(message.origin(), Origin::Live);
    assert!(sent.fallbacks.is_empty());
    assert!(matches!(
        bob.state().user(&alice_nick).unwrap().account(),
        Account::LoggedIn(observed) if observed == &account
    ));
    bob.expect_none(Event::as_message).await?;
    Ok(())
}

/// confirms away state locally and notifies channel peers of both transitions
#[scenario(
    requires(caps = ["away-notify"]),
    covers = ["base.away.self-ack", "ircv3.away-notify"]
)]
async fn away_notify(context: Scenario) -> papo_testing::Result<()> {
    let mut bob = context.connect("bob").await?;
    let alice = context.connect("alice").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    bob.join(&channel).await?;
    alice.join(&channel).await?;
    let away = "reviewing the IRCv3 specification";
    alice.set_away(away).await?;
    let changed = bob.wait_for(Event::as_away_changed).await?;

    assert_eq!(changed.nick, alice_nick);
    assert_eq!(changed.message.as_deref(), Some(away));
    assert!(alice.state().away());
    assert_eq!(alice.state().away_message(), Some(away));
    assert_eq!(
        bob.state().user(&alice_nick).unwrap().away(),
        &Away::Away {
            message: Some(away.to_owned())
        }
    );

    alice.clear_away().await?;
    let returned = bob.wait_for(Event::as_away_changed).await?;

    assert_eq!(returned.nick, alice_nick);
    assert_eq!(returned.message, None);
    assert!(!alice.state().away());
    assert_eq!(alice.state().away_message(), None);
    assert_eq!(
        bob.state().user(&alice_nick).unwrap().away(),
        &Away::Present
    );
    bob.expect_none(Event::as_away_changed).await?;
    Ok(())
}

/// updates real-name state for the sender and every shared-channel peer
#[scenario(requires(caps = ["setname"]), covers = ["ircv3.setname"])]
async fn setname(context: Scenario) -> papo_testing::Result<()> {
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let alice_nick = alice.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let realname = "Papo Conformance Client";
    alice.set_realname(realname).await?;
    let own = alice.wait_for(Event::as_realname_changed).await?;
    let observed = bob.wait_for(Event::as_realname_changed).await?;

    assert_eq!(own.nick, alice_nick);
    assert_eq!(own.realname, realname);
    assert_eq!(observed.nick, alice_nick);
    assert_eq!(observed.realname, realname);
    assert_eq!(
        alice.state().user(&alice_nick).unwrap().realname(),
        Some(realname)
    );
    assert_eq!(
        bob.state().user(&alice_nick).unwrap().realname(),
        Some(realname)
    );
    alice.expect_none(Event::as_realname_changed).await?;
    bob.expect_none(Event::as_realname_changed).await?;
    Ok(())
}

/// updates username and host state for the target and a shared-channel peer
#[scenario(
    requires(caps = ["chghost"]),
    covers = ["base.oper.success", "ircv3.chghost"]
)]
async fn chghost(context: Scenario) -> papo_testing::Result<()> {
    let Ok(oper) = std::env::var("PAPO_TEST_OPER") else {
        return context.skip("PAPO_TEST_OPER is not set up");
    };
    let Some((oper_name, password)) = oper.split_once(':') else {
        return Err("PAPO_TEST_OPER must contain name:password".into());
    };
    let mut alice = context.connect("alice").await?;
    let mut bob = context.connect("bob").await?;
    let channel = context.channel_name();
    let bob_nick = bob.nick();
    alice.join(&channel).await?;
    bob.join(&channel).await?;
    let username = bob
        .state()
        .user(&bob_nick)
        .unwrap()
        .username()
        .unwrap()
        .to_owned();

    alice.oper(oper_name, password).await?;
    alice
        .send_command("CHGHOST", [bob_nick.as_ref(), "conformance.example"])
        .await?;
    let own = bob.wait_for(Event::as_host_changed).await?;
    let observed = alice.wait_for(Event::as_host_changed).await?;

    for changed in [&own, &observed] {
        assert_eq!(changed.nick, bob_nick);
        assert_eq!(changed.user, username);
        assert_eq!(changed.host, "conformance.example");
    }
    for peer in [&alice, &bob] {
        let state = peer.state();
        let user = state.user(&bob_nick).unwrap();
        assert_eq!(user.username(), Some(username.as_str()));
        assert_eq!(user.host(), Some("conformance.example"));
    }
    alice.expect_none(Event::as_host_changed).await?;
    bob.expect_none(Event::as_host_changed).await?;
    Ok(())
}

/// reports monitored users with extended-monitor enabled
#[scenario(requires(
    caps = ["extended-monitor", "away-notify"],
    isupport = ["MONITOR"]
))]
async fn monitor(context: Scenario) -> ScenarioResult {
    let observer_nick = context.unique();
    let target_nick = context.unique();
    let mut observer = context.client(&observer_nick).await?;
    let target = context.client(&target_nick).await?;
    observer
        .client
        .monitor()
        .add([target_nick.clone()])
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut observer.events,
            |event| matches!(event, Event::MonitorOnline(online) if online.nick == target_nick.as_str()),
            EXPECT_TIMEOUT,
        )
        .await
        .map_err(|_| Failed::from("timed out waiting for MONITOR presence"))?;
    close_pair(observer, target).await
}

/// marks a client with the advertised bot mode
#[scenario(requires(isupport = ["BOT"]))]
async fn bot_mode(context: Scenario) -> ScenarioResult {
    let nick = context.unique();
    let mut connection = context.client(&nick).await?;
    let registered = connection.welcome.nick.clone();
    connection
        .client
        .send_command("MODE", [registered.as_str(), "+B"])
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut connection.events,
            |event| matches!(event, Event::ModeChanged(changed) if changed.target == registered.as_str()),
            EXPECT_TIMEOUT,
        )
        .await?;
    let whois = connection
        .client
        .whois(&registered)
        .await
        .map_err(Failed::from)?;
    if !whois.is_bot {
        return Err("WHOIS did not report the advertised bot mode".into());
    }
    connection.close().await
}
