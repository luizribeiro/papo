use papo::{Away, Event};
use papo_conformance_macros::scenario;

use super::support::conversation_name;
use crate::{Peer, Scenario};

fn has_account_sessions() -> bool {
    matches!(
        std::env::var("PAPO_TEST_PROFILE").as_deref(),
        Err(_) | Ok("ergo-default" | "ergo-persistent" | "soju-ergo")
    )
}

async fn wait_for_channel(peer: &mut Peer, channel: &str) -> papo_testing::Result<()> {
    if peer
        .state()
        .channel(channel)
        .is_some_and(papo::Channel::is_synced)
    {
        return Ok(());
    }
    peer.wait_for(|event| {
        event
            .as_channel_synced()
            .filter(|synced| synced.channel.as_ref() == channel)
    })
    .await?;
    Ok(())
}

/// routes one session's channel send to its sibling exactly once as an own message
#[scenario(
    requires(caps = ["echo-message"]),
    covers = ["multidevice.message-fanout"]
)]
async fn multidevice_message_fanout(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (mut desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s.connect_account_session("alice-phone", account).await?;
    wait_for_channel(&mut phone, &channel).await?;

    let text = "release update from the desktop session";
    desktop.privmsg(&channel, text).await?;
    let received = phone
        .wait_for(|event| {
            event
                .as_own_message()
                .filter(|message| message.text == text)
        })
        .await?;
    assert_eq!(received.authorship, papo::Authorship::Own);
    assert_eq!(
        conversation_name(&received.conversation),
        Some(channel.as_str())
    );
    assert_eq!(received.return_address(), channel);
    phone
        .expect_none(|event| {
            event
                .as_own_message()
                .filter(|message| message.text == text)
        })
        .await?;
    desktop
        .expect_none(|event| {
            event
                .as_own_message()
                .filter(|message| message.text == text)
        })
        .await?;
    Ok(())
}

/// applies one session's away state to sibling sessions and other users
#[scenario(
    requires(caps = ["away-notify"]),
    covers = ["multidevice.away-shared-state"]
)]
async fn multidevice_away_shared_state(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s.connect_account_session("alice-phone", account).await?;
    wait_for_channel(&mut phone, &channel).await?;
    let mut observer = s.connect("bob").await?;
    observer.join(&channel).await?;
    let nick = desktop.nick();
    phone
        .wait_for(|event| {
            event
                .as_joined()
                .filter(|joined| joined.nick == observer.nick())
        })
        .await?;

    let reason = "reviewing the production rollout";
    desktop.set_away(reason).await?;
    let away = Away::__away_value(Some(reason.to_owned()));
    assert_eq!(desktop.state().away(), &away);
    if std::env::var("PAPO_TEST_PROFILE").as_deref() == Ok("soju-ergo") {
        phone.expect_none(Event::as_away_changed).await?;
        observer.expect_none(Event::as_away_changed).await?;
        assert_eq!(phone.state().away(), &Away::Present);
        assert_eq!(observer.state().user(&nick).unwrap().away(), &Away::Present);
        return Ok(());
    }
    let sibling = phone
        .wait_for(|event| {
            event
                .as_away_changed()
                .filter(|changed| changed.nick == nick)
        })
        .await?;
    let visible = observer
        .wait_for(|event| {
            event
                .as_away_changed()
                .filter(|changed| changed.nick == nick)
        })
        .await?;
    assert_eq!(sibling.away, away);
    assert_eq!(visible.away, away);
    assert_eq!(phone.state().away(), &away);
    assert_eq!(observer.state().user(&nick).unwrap().away(), &away);
    phone.expect_none(Event::as_away_changed).await?;
    observer.expect_none(Event::as_away_changed).await?;
    Ok(())
}
