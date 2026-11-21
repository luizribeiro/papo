use papo::{Away, Error, Event, Party, Refusal};
use papo_conformance_macros::scenario;

use super::support::{
    any_message_text, confirmed_line, confirmed_redaction, conversation_name, join_seen_by,
    latest_history_with, reaction_or_skip, restricted_line, restricted_message, restricted_text,
    wait_for_channel,
};
use crate::Scenario;

fn has_account_sessions() -> bool {
    matches!(
        std::env::var("PAPO_TEST_PROFILE").as_deref(),
        Err(_) | Ok("ergo-default" | "ergo-persistent" | "soju-ergo")
    )
}

fn chat_text<'event>(event: &'event Event, text: &str) -> Option<&'event papo::Message> {
    event
        .as_message()
        .filter(|message| message.raw_text == text)
}

/// routes one session's channel send to its sibling exactly once as `Party::Me`
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
        .wait_for(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    assert_eq!(received.actor_party, papo::Party::Me);
    assert_eq!(
        conversation_name(received.conversation()),
        Some(channel.clone())
    );
    assert_eq!(received.return_address(), channel.as_str());
    phone
        .expect_none(|event| event.as_message().filter(|message| message.text == text))
        .await?;
    desktop
        .expect_none(|event| event.as_message().filter(|message| message.text == text))
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
    wait_for_channel(&mut observer, &channel).await?;
    // The member's presence comes from the join-time WHO. A bouncer forwards that WHO
    // upstream but answers the later AWAY and PING fences itself, so only this event
    // orders the reply before the snapshot below.
    observer
        .wait_for(|event| {
            event
                .as_members_refreshed()
                .filter(|refreshed| refreshed.channel.name().as_str() == channel)
        })
        .await?;
    let nick = desktop.snapshot().nick().clone();
    phone
        .wait_for(|event| {
            event.as_joined().filter(|joined| {
                joined.actor.nick().as_str() == observer.snapshot().nick().as_str()
            })
        })
        .await?;

    let reason = "reviewing the production rollout";
    desktop.set_away(reason).await?;
    let away = Away::__away_value(Some(reason.to_owned()));
    assert_eq!(desktop.snapshot().away(), &away);
    if std::env::var("PAPO_TEST_PROFILE").as_deref() == Ok("soju-ergo") {
        phone.expect_none(Event::as_away_changed).await?;
        observer.expect_none(Event::as_away_changed).await?;
        assert_eq!(phone.snapshot().away(), &Away::Present);
        assert_eq!(
            super::user(&observer.snapshot(), nick.as_str())
                .unwrap()
                .away(),
            &Away::Present
        );
        return Ok(());
    }
    let sibling = phone
        .wait_for(|event| {
            event
                .as_away_changed()
                .filter(|changed| changed.actor.nick() == nick.as_str())
        })
        .await?;
    let visible = observer
        .wait_for(|event| {
            event
                .as_away_changed()
                .filter(|changed| changed.actor.nick() == nick.as_str())
        })
        .await?;
    assert_eq!(sibling.new, away);
    assert_eq!(visible.new, away);
    assert_eq!(sibling.actor_party, Party::Me);
    assert_eq!(visible.actor_party, Party::Other);
    assert_eq!(phone.snapshot().away(), &away);
    assert_eq!(
        super::user(&observer.snapshot(), nick.as_str())
            .unwrap()
            .away(),
        &away
    );
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
    let (mut desktop, account) = s.connect_as_account_with_key("alice").await?;
    let channel = s.channel_name();
    desktop.join(&channel).await?;
    let mut phone = s.connect_account_session("alice-phone", account).await?;
    wait_for_channel(&mut phone, &channel).await?;

    phone
        .privmsg(&channel, "multi-device read-marker point")
        .await?;
    let message = desktop.wait_for(Event::as_message).await?;
    let time = message
        .server_time
        .expect("read markers require server time");
    assert!(matches!(
        desktop.mark_read(&channel).await?,
        papo::MarkReadOutcome::Advanced { marker, .. } if marker == time
    ));
    let marker = phone
        .wait_for(|event| {
            event.as_read_marker_changed().filter(|marker| {
                conversation_name(&marker.conversation) == Some(channel.clone())
                    && marker.marker == papo::ReadMarker::ReadUntil(time)
            })
        })
        .await?;
    assert_eq!(marker.marker, papo::ReadMarker::ReadUntil(time));
    assert_eq!(
        phone.snapshot().read_marker(&channel),
        Some(papo::ReadMarker::ReadUntil(time))
    );
    phone
        .expect_none(|event| {
            event
                .as_read_marker_changed()
                .filter(|marker| marker.marker == papo::ReadMarker::ReadUntil(time))
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
    let (sign, original) = restricted_message(
        phone
            .wait_for(|event| restricted_text(event, "operators reviewing the deployment"))
            .await?,
    );
    let (id, sent_text, _) = confirmed_line(&sent);
    assert_eq!(sent_text, "operators reviewing the deployment");
    assert_eq!(original.msgid(), Some(id));
    assert_eq!(sign, '@');
    assert_eq!(original.return_address(), target.as_str());
    observer
        .expect_none(|event| any_message_text(event, "operators reviewing the deployment"))
        .await?;

    desktop
        .reply(&sent, "operators approve the deployment")
        .await?;
    let (sign, reply) = restricted_message(
        phone
            .wait_for(|event| restricted_text(event, "operators approve the deployment"))
            .await?,
    );
    assert_eq!(reply.in_reply_to.as_ref(), Some(id));
    assert_eq!(sign, '@');
    assert_eq!(reply.return_address(), target.as_str());
    observer
        .expect_none(|event| any_message_text(event, "operators approve the deployment"))
        .await?;

    reaction_or_skip(desktop.react(&sent, "🛡️").await)?;
    let added = phone
        .wait_for(|event| restricted_line(event, papo::RestrictedLine::as_reacted))
        .await?;
    assert_eq!(added.audience.symbol(), '@');
    let added = added.line.as_reacted().expect("reaction line");
    assert_eq!(&added.reacts_to, id);
    assert_eq!(added.reaction, "🛡️");
    assert_eq!(added.return_address(), target.as_str());
    observer
        .expect_none(|event| {
            restricted_line(event, papo::RestrictedLine::as_reacted)
                .map(|_| event)
                .or_else(|| event.as_reacted().map(|_| event))
        })
        .await?;

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
                .as_message()
                .filter(|message| message.text == "deployment note to remove")
        })
        .await?;
    let (id, sent_text, _) = confirmed_line(&sent);
    assert_eq!(sent_text, "deployment note to remove");
    let outcome = desktop.redact(&sent, "deployment superseded").await?;
    let removed = phone.wait_for(Event::as_redacted).await?;
    let (redacts, echo, echo_time) = confirmed_redaction(&outcome);
    assert_eq!(redacts, id);
    assert_eq!(removed.msgid(), Some(echo));
    assert_eq!(removed.server_time, echo_time);
    assert_eq!(&removed.redacts, id);
    assert_eq!(removed.reason.as_deref(), Some("deployment superseded"));
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
    let old = desktop.snapshot().nick().clone();
    let new = format!("m{old}");

    let outcome = desktop.set_nick(&new).await;
    assert!(matches!(outcome, Err(Error::Server(_))));
    desktop.expect_none(Event::as_nick_changed).await?;
    phone.expect_none(Event::as_nick_changed).await?;
    tablet.expect_none(Event::as_nick_changed).await?;
    for peer in [&desktop, &phone, &tablet] {
        assert_eq!(peer.snapshot().nick(), old.as_str());
        let members = peer.snapshot();
        assert!(
            members
                .__channel_local(&channel)
                .expect("joined channel remains in state")
                .member(old.as_str())
                .unwrap()
                .is_some()
        );
        assert!(
            members
                .__channel_local(&channel)
                .unwrap()
                .member(new.as_str())
                .unwrap()
                .is_none()
        );
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
    let nick = desktop.snapshot().nick().clone();

    sender
        .send_raw(papo::Line::new("PRIVMSG").with_params([nick.as_str(), "\u{1}VERSION\u{1}"]))
        .await?;
    for peer in [&mut desktop, &mut phone, &mut tablet] {
        let query = peer
            .wait_for(|event| {
                event
                    .as_ctcp_query()
                    .filter(|query| query.command == "VERSION")
            })
            .await?;
        assert_eq!(query.return_address(), sender.snapshot().nick().as_str());
    }
    for _ in 0..3 {
        let reply = sender
            .wait_for(|event| {
                event.as_ctcp_reply().filter(|reply| {
                    reply.command == "VERSION" && reply.actor.nick().as_str() == nick.as_str()
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
    phone.quit().await?;
    phone.wait_for(Event::as_disconnected).await?;

    let texts = [
        "deployment started while the phone was offline",
        "deployment completed while the phone was offline",
    ];
    for text in texts {
        desktop.privmsg(&channel, text).await?;
    }

    let mut reconnected = s.connect_account_session("alice-phone", account).await?;
    let page = latest_history_with(&reconnected, &channel, texts[1]).await?;
    assert_eq!(page.reason, papo::HistoryReason::Requested);
    assert_eq!(conversation_name(&page.conversation), Some(channel.clone()));
    let own = page
        .items
        .iter()
        .filter_map(Event::as_message)
        .filter(|message| message.actor_party == Party::Me)
        .filter(|message| texts.contains(&message.text.as_str()))
        .count();
    assert_eq!(own, 2);
    let replayed = page
        .items
        .iter()
        .filter_map(Event::as_message)
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
    assert_eq!(typing.actor_party, papo::Party::Me);
    assert_eq!(
        crate::source_nick(&typing.actor),
        Some(desktop.snapshot().nick().as_str())
    );
    assert_eq!(typing.state, papo::Typing::Active);
    assert_eq!(
        conversation_name(typing.conversation()),
        Some(channel.clone())
    );
    assert_eq!(
        phone
            .snapshot()
            .typing(typing.conversation())
            .filter(|typer| typer.nick() == desktop.snapshot().nick().as_str())
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
    let mut sender = s.connect("bob").await?;
    let text = "can every device review the release?";
    let channel = s.channel_name();
    sender.join(&channel).await?;
    wait_for_channel(&mut sender, &channel).await?;
    join_seen_by(&desktop, &mut sender, &channel).await?;
    let mut phone = s
        .connect_account_session("alice-phone", account.clone())
        .await?;
    let mut tablet = s.connect_account_session("alice-tablet", account).await?;
    wait_for_channel(&mut phone, &channel).await?;
    wait_for_channel(&mut tablet, &channel).await?;

    let sent = sender
        .privmsg(desktop.snapshot().nick().as_str(), text)
        .await?;
    let on_desktop = desktop.wait_for(|event| chat_text(event, text)).await?;
    let on_phone = phone.wait_for(|event| chat_text(event, text)).await?;
    let on_tablet = tablet.wait_for(|event| chat_text(event, text)).await?;
    let (id, sent_text, _) = confirmed_line(&sent);
    assert_eq!(sent_text, text);
    for message in [&on_desktop, &on_phone, &on_tablet] {
        assert_eq!(message.msgid(), Some(id));
        assert_eq!(message.actor_party, papo::Party::Other);
        assert_eq!(
            conversation_name(message.conversation()),
            Some(sender.snapshot().nick().to_string())
        );
        assert_eq!(message.return_address(), sender.snapshot().nick().as_str());
    }
    for peer in [&mut desktop, &mut phone, &mut tablet] {
        peer.expect_none(|event| chat_text(event, text)).await?;
    }

    phone.reply(&on_phone, "reviewed from the phone").await?;
    let sibling_reply = desktop
        .wait_for(|event| chat_text(event, "reviewed from the phone"))
        .await?;
    assert_eq!(sibling_reply.actor_party, papo::Party::Me);
    assert_eq!(sibling_reply.in_reply_to.as_ref(), Some(id));
    assert_eq!(
        conversation_name(sibling_reply.conversation()),
        Some(sender.snapshot().nick().to_string())
    );
    assert_eq!(
        sibling_reply.return_address(),
        sender.snapshot().nick().as_str()
    );
    let received_reply = sender
        .wait_for(|event| chat_text(event, "reviewed from the phone"))
        .await?;
    assert_eq!(received_reply.in_reply_to.as_ref(), Some(id));
    assert_eq!(
        received_reply.return_address(),
        desktop.snapshot().nick().as_str()
    );
    desktop
        .expect_none(|event| chat_text(event, "reviewed from the phone"))
        .await?;
    Ok(())
}
