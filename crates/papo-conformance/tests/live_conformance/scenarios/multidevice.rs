use std::time::{Duration, SystemTime};

use papo::{Authorship, Away, Error, Event, Refusal};
use papo_conformance_macros::scenario;

use super::support::conversation_name;
use crate::{Peer, Scenario};

fn has_account_sessions() -> bool {
    matches!(
        std::env::var("PAPO_TEST_PROFILE").as_deref(),
        Err(_) | Ok("ergo-default" | "ergo-persistent" | "soju-ergo")
    )
}

fn chat_text<'event>(event: &'event Event, text: &str) -> Option<&'event papo::Message> {
    event
        .as_message()
        .or_else(|| event.as_own_message())
        .filter(|message| message.raw_text == text)
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
    assert_eq!(received.return_address(), channel.as_str());
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
                .filter(|joined| joined.actor.nick().as_ref() == observer.nick().as_ref())
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
        assert_eq!(
            observer.state().user(nick.as_ref()).unwrap().away(),
            &Away::Present
        );
        return Ok(());
    }
    let sibling = phone
        .wait_for(|event| {
            event
                .as_away_changed()
                .filter(|changed| changed.actor.nick() == nick.as_ref())
        })
        .await?;
    let visible = observer
        .wait_for(|event| {
            event
                .as_away_changed()
                .filter(|changed| changed.actor.nick() == nick.as_ref())
        })
        .await?;
    assert_eq!(sibling.new, away);
    assert_eq!(visible.new, away);
    assert_eq!(sibling.authorship, Authorship::Own);
    assert_eq!(visible.authorship, Authorship::Other);
    assert_eq!(phone.state().away(), &away);
    assert_eq!(observer.state().user(nick.as_ref()).unwrap().away(), &away);
    phone.expect_none(Event::as_away_changed).await?;
    observer.expect_none(Event::as_away_changed).await?;
    Ok(())
}

/// broadcasts a read marker change to another attached session
#[scenario(
    requires(caps = ["draft/read-marker"]),
    covers = ["multidevice.read-marker-fanout"]
)]
async fn multidevice_read_marker_fanout(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s.connect_account_session("alice-phone", account).await?;
    wait_for_channel(&mut phone, &channel).await?;

    let time = SystemTime::UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000);
    assert_eq!(desktop.mark_read(&channel, time).await?, Some(time));
    let marker = phone
        .wait_for(|event| {
            event.as_read_marker_changed().filter(|marker| {
                conversation_name(&marker.conversation) == Some(channel.as_str())
                    && marker.read_until == Some(time)
            })
        })
        .await?;
    assert_eq!(marker.read_until, Some(time));
    assert_eq!(
        phone.state().read_marker(&channel),
        Some(papo::ReadMarker::ReadUntil(time))
    );
    phone
        .expect_none(|event| {
            event
                .as_read_marker_changed()
                .filter(|marker| marker.read_until == Some(time))
        })
        .await?;
    Ok(())
}

/// preserves an operators-only audience across sibling-visible message operations
#[scenario(
    requires(caps = [
        "draft/message-redaction",
        "echo-message",
        "labeled-response",
        "message-tags"
    ], isupport = ["STATUSMSG"]),
    covers = ["multidevice.ops-message-operations"]
)]
async fn multidevice_ops_message_operations(s: Scenario) -> papo_testing::Result<()> {
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
    let target = format!("@{channel}");

    let sent = desktop
        .privmsg(&target, "operators reviewing the deployment")
        .await?;
    let original = phone
        .wait_for(|event| {
            event
                .as_own_message()
                .filter(|message| message.text == "operators reviewing the deployment")
        })
        .await?;
    let id = sent.msgid().expect("message has an identifier");
    assert_eq!(original.msgid.as_ref(), Some(id));
    assert_eq!(original.status.map(papo::Prefix::symbol), Some('@'));
    assert_eq!(original.return_address(), target.as_str());
    observer
        .expect_none(|event| {
            event
                .as_message()
                .filter(|message| message.text == "operators reviewing the deployment")
        })
        .await?;

    desktop
        .reply(&sent, "operators approve the deployment")
        .await?;
    let reply = phone
        .wait_for(|event| {
            event
                .as_own_message()
                .filter(|message| message.text == "operators approve the deployment")
        })
        .await?;
    assert_eq!(reply.in_reply_to.as_ref(), Some(id));
    assert_eq!(reply.status.map(papo::Prefix::symbol), Some('@'));
    assert_eq!(reply.return_address(), target.as_str());
    observer
        .expect_none(|event| {
            event
                .as_message()
                .filter(|message| message.text == "operators approve the deployment")
        })
        .await?;

    desktop.react(&sent, "🛡️").await?;
    let added = phone.wait_for(Event::as_reacted).await?;
    assert_eq!(&added.reacts_to, id);
    assert_eq!(added.reaction, "🛡️");
    assert_eq!(added.status.map(papo::Prefix::symbol), Some('@'));
    assert_eq!(added.return_address(), target.as_str());
    observer.expect_none(Event::as_reacted).await?;

    let Error::Server(refusal) = desktop
        .redact(&sent, "deployment superseded")
        .await
        .expect_err("servers reject status-prefixed REDACT targets")
    else {
        panic!("status-prefixed REDACT did not return a server refusal");
    };
    assert_eq!(refusal.refusal(), Refusal::NoSuchNick);
    phone.expect_none(Event::as_redacted).await?;
    observer.expect_none(Event::as_redacted).await?;

    let sent = desktop
        .privmsg(&channel, "deployment note to remove")
        .await?;
    phone
        .wait_for(|event| {
            event
                .as_own_message()
                .filter(|message| message.text == "deployment note to remove")
        })
        .await?;
    let id = sent.msgid().expect("message has an identifier");
    desktop.redact(&sent, "deployment superseded").await?;
    let removed = phone.wait_for(Event::as_redacted).await?;
    assert_eq!(&removed.redacts, id);
    assert_eq!(removed.reason.as_deref(), Some("deployment superseded"));
    assert_eq!(removed.status, None);
    assert_eq!(removed.return_address(), channel.as_str());
    Ok(())
}

/// synchronizes an accepted nickname or preserves state after account enforcement
#[scenario(covers = ["multidevice.nick-shared-state"])]
async fn multidevice_nick_shared_state(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (mut desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s
        .connect_account_session("alice-phone", account.clone())
        .await?;
    wait_for_channel(&mut phone, &channel).await?;
    let mut tablet = s.connect_account_session("alice-tablet", account).await?;
    wait_for_channel(&mut tablet, &channel).await?;
    let old = desktop.nick();
    let new = format!("m{old}");

    let outcome = desktop.set_nick(&new).await;
    if std::env::var("PAPO_TEST_PROFILE").as_deref() == Ok("soju-ergo") {
        outcome?;
        for changed in [
            desktop.wait_for(Event::as_nick_changed).await?,
            phone.wait_for(Event::as_nick_changed).await?,
            tablet.wait_for(Event::as_nick_changed).await?,
        ] {
            assert_eq!(changed.actor.nick(), old.as_ref());
            assert_eq!(changed.new.as_ref(), new);
            assert_eq!(changed.authorship, papo::Authorship::Own);
        }
        for peer in [&desktop, &phone, &tablet] {
            assert_eq!(peer.state().nick().as_ref(), new);
        }
        return Ok(());
    }
    assert!(matches!(outcome, Err(Error::Server(_))));
    desktop.expect_none(Event::as_nick_changed).await?;
    phone.expect_none(Event::as_nick_changed).await?;
    tablet.expect_none(Event::as_nick_changed).await?;
    for peer in [&desktop, &phone, &tablet] {
        assert_eq!(peer.state().nick(), old.as_ref());
        let members = peer.state();
        assert!(
            members
                .channel(&channel)
                .expect("joined channel remains in state")
                .member(old.as_ref())
                .is_some()
        );
        assert!(members.channel(&channel).unwrap().member(&new).is_none());
    }
    Ok(())
}

/// delivers a private CTCP query to every attached session and observes each automatic reply
#[scenario(covers = ["multidevice.ctcp-session-replies"])]
async fn multidevice_ctcp_session_replies(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (mut desktop, account) = s.connect_as_account_with_key("alice").await?;
    let mut phone = s
        .connect_account_session("alice-phone", account.clone())
        .await?;
    let mut tablet = s.connect_account_session("alice-tablet", account).await?;
    let mut sender = s.connect("bob").await?;
    let nick = desktop.nick();

    sender
        .send_raw(papo::Line::new("PRIVMSG").with_params([nick.as_ref(), "\u{1}VERSION\u{1}"]))
        .await?;
    for peer in [&mut desktop, &mut phone, &mut tablet] {
        let query = peer
            .wait_for(|event| {
                event
                    .as_ctcp_query()
                    .filter(|query| query.command == "VERSION")
            })
            .await?;
        assert_eq!(query.return_address(), sender.nick().as_ref());
    }
    for _ in 0..3 {
        let reply = sender
            .wait_for(|event| {
                event.as_ctcp_reply().filter(|reply| {
                    reply.command == "VERSION" && reply.actor.nick().as_ref() == nick.as_ref()
                })
            })
            .await?;
        assert!(
            reply
                .params
                .as_deref()
                .is_some_and(|params| params.starts_with("papo "))
        );
    }
    sender.expect_none(Event::as_ctcp_reply).await?;
    Ok(())
}

/// retrieves messages missed by a detached session once in one history page
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["multidevice.reconnect-catchup"]
)]
async fn multidevice_reconnect_catchup(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s
        .connect_account_session("alice-phone", account.clone())
        .await?;
    wait_for_channel(&mut phone, &channel).await?;
    phone.quit("switching devices").await?;
    phone.wait_for(Event::as_disconnected).await?;

    let texts = [
        "deployment started while the phone was offline",
        "deployment completed while the phone was offline",
    ];
    for text in texts {
        desktop.privmsg(&channel, text).await?;
    }

    let mut reconnected = s.connect_account_session("alice-phone", account).await?;
    let page = reconnected.history(&channel).latest().limit(10).await?;
    assert_eq!(page.reason, papo::HistoryReason::Requested);
    assert_eq!(
        conversation_name(&page.conversation),
        Some(channel.as_str())
    );
    let own = page
        .items
        .iter()
        .filter_map(Event::as_own_message)
        .filter(|message| texts.contains(&message.text.as_str()))
        .count();
    if std::env::var("PAPO_TEST_PROFILE").as_deref() == Ok("soju-ergo") {
        assert_eq!(own, 0);
    } else {
        assert_eq!(own, 2);
    }
    let replayed = page
        .items
        .iter()
        .filter_map(|event| event.as_own_message().or_else(|| event.as_message()))
        .filter(|message| texts.contains(&message.text.as_str()))
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(replayed, texts);
    for text in texts {
        reconnected
            .expect_none(|event| chat_text(event, text))
            .await?;
    }
    Ok(())
}

/// classifies sibling typing as own and keeps it out of the active-typers state
#[scenario(
    requires(caps = ["message-tags"]),
    covers = ["multidevice.typing-own-state"]
)]
async fn multidevice_typing_own_state(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s.connect_account_session("alice-phone", account).await?;
    wait_for_channel(&mut phone, &channel).await?;

    desktop.typing(&channel, papo::Typing::Active).await?;
    let typing = phone.wait_for(Event::as_typing_changed).await?;
    assert_eq!(typing.authorship, papo::Authorship::Own);
    assert_eq!(
        crate::source_nick(&typing.actor),
        Some(desktop.nick().as_ref())
    );
    assert_eq!(typing.state, papo::Typing::Active);
    assert_eq!(
        conversation_name(&typing.conversation),
        Some(channel.as_str())
    );
    assert_eq!(
        phone
            .state()
            .typing(&typing.conversation)
            .filter(|typer| typer.nick() == desktop.nick().as_ref())
            .count(),
        0
    );
    phone.expect_none(Event::as_typing_changed).await?;
    Ok(())
}

/// fans a direct message to every session and threads a sibling reply on the others
#[scenario(
    requires(caps = ["echo-message", "labeled-response", "message-tags"]),
    covers = ["multidevice.dm-fanout-thread"]
)]
async fn multidevice_dm_fanout_thread(s: Scenario) -> papo_testing::Result<()> {
    if !has_account_sessions() {
        return s.skip("the selected profile does not support concurrent account sessions");
    }
    let (mut desktop, account) = s.connect_as_account_with_key("alice").await?;
    let mut phone = s
        .connect_account_session("alice-phone", account.clone())
        .await?;
    let mut tablet = s.connect_account_session("alice-tablet", account).await?;
    let mut sender = s.connect("bob").await?;
    let text = "can every device review the release?";

    let sent = sender.privmsg(desktop.nick().as_ref(), text).await?;
    let on_desktop = desktop.wait_for(|event| chat_text(event, text)).await?;
    let on_phone = phone.wait_for(|event| chat_text(event, text)).await?;
    let on_tablet = tablet.wait_for(|event| chat_text(event, text)).await?;
    let id = sent.msgid().expect("direct message has an identifier");
    for message in [&on_desktop, &on_phone, &on_tablet] {
        assert_eq!(message.msgid.as_ref(), Some(id));
        assert_eq!(message.authorship, papo::Authorship::Other);
        assert_eq!(
            conversation_name(&message.conversation),
            Some(sender.nick().as_ref())
        );
        assert_eq!(message.return_address(), sender.nick().as_ref());
    }
    for peer in [&mut desktop, &mut phone, &mut tablet] {
        peer.expect_none(|event| chat_text(event, text)).await?;
    }

    phone.reply(&on_phone, "reviewed from the phone").await?;
    let sibling_reply = desktop
        .wait_for(|event| chat_text(event, "reviewed from the phone"))
        .await?;
    assert_eq!(sibling_reply.authorship, papo::Authorship::Own);
    assert_eq!(sibling_reply.in_reply_to.as_ref(), Some(id));
    assert_eq!(
        conversation_name(&sibling_reply.conversation),
        Some(sender.nick().as_ref())
    );
    assert_eq!(sibling_reply.return_address(), sender.nick().as_ref());
    let received_reply = sender
        .wait_for(|event| chat_text(event, "reviewed from the phone"))
        .await?;
    assert_eq!(received_reply.in_reply_to.as_ref(), Some(id));
    assert_eq!(received_reply.return_address(), desktop.nick().as_ref());
    desktop
        .expect_none(|event| chat_text(event, "reviewed from the phone"))
        .await?;
    Ok(())
}
