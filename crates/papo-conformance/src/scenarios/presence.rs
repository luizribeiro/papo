use libtest_mimic::Failed;
use papo::{Account, Event, Origin, Source};
use papo_conformance_macros::scenario;

use super::support::{EXPECT_TIMEOUT, close_pair, joined_pair};
use crate::{Scenario, ScenarioResult, required_environment};

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

/// notifies channel peers of away state
#[scenario(requires(caps = ["away-notify"]))]
async fn away_notify(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, _channel) = joined_pair(&context).await?;
    let away = "reviewing the IRCv3 specification";
    sender.client.set_away(away).await.map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::AwayChanged(changed) if changed.nick == sender.welcome.nick.as_str() && changed.message.as_deref() == Some(away)),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// notifies channel peers of real-name changes
#[scenario(requires(caps = ["setname"]))]
async fn setname(context: Scenario) -> ScenarioResult {
    let (mut receiver, sender, _channel) = joined_pair(&context).await?;
    let realname = "Papo Conformance Client";
    sender
        .client
        .set_realname(realname)
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut receiver.events,
            |event| matches!(event, Event::RealnameChanged(changed) if changed.nick == sender.welcome.nick.as_str() && changed.realname == realname),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(receiver, sender).await
}

/// notifies channel peers of operator host changes
#[scenario(requires(caps = ["chghost"]))]
async fn chghost(context: Scenario) -> ScenarioResult {
    let oper = match required_environment("PAPO_TEST_OPER") {
        Ok(oper) => oper,
        Err(ignored) => return Ok(ignored),
    };
    let Some((oper_name, password)) = oper.split_once(':') else {
        return Err("PAPO_TEST_OPER must contain name:password".into());
    };
    let (mut observer, operator, _channel) = joined_pair(&context).await?;
    operator
        .client
        .oper(oper_name, password)
        .await
        .map_err(Failed::from)?;
    operator
        .client
        .send_command(
            "CHGHOST",
            [observer.welcome.nick.as_str(), "conformance.example"],
        )
        .await
        .map_err(Failed::from)?;
    context
        .expect(
            &mut observer.events,
            |event| matches!(event, Event::HostChanged(changed) if changed.nick == observer.welcome.nick.as_str() && changed.host == "conformance.example"),
            EXPECT_TIMEOUT,
        )
        .await?;
    close_pair(observer, operator).await
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
