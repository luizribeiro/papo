use papo::{Event, event::StandardReplyKind};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{confirmed_line, conversation_name};

/// a labelled WHOIS resolves to the requested user
#[scenario(
    requires(caps = ["labeled-response"]),
    covers = [
        "ircv3.labeled-response.single-line",
        "core.labeled-response.correlation"
    ]
)]
async fn labeled_response(s: Scenario) -> papo_testing::Result<()> {
    let alice = s.connect("alice").await?;
    let whois = alice.whois(alice.nick().clone()).await?;
    assert_eq!(whois.nick, alice.nick());
    assert!(
        whois
            .username
            .as_deref()
            .is_some_and(|username| !username.is_empty())
    );
    assert!(
        whois
            .realname
            .as_deref()
            .is_some_and(|realname| !realname.is_empty())
    );
    assert!(whois.host.is_some());
    assert_eq!(alice.state().nick(), &whois.nick);
    Ok(())
}

/// a labelled history request resolves its complete batch
#[scenario(
    requires(caps = ["batch", "draft/chathistory", "labeled-response"]),
    covers = [
        "ircv3.batch.open-close",
        "ircv3.labeled-response.batch-wrapped"
    ]
)]
async fn batch(s: Scenario) -> papo_testing::Result<()> {
    let writer = s.connect("writer").await?;
    let channel = s.channel_name();
    writer.join(&channel).await?;
    let sent = writer
        .privmsg(&channel, "batch correlation deployment")
        .await?;

    let reader = s.connect("reader").await?;
    reader.join(&channel).await?;
    let page = reader.history(&channel).latest().limit(10).await?;
    assert_eq!(
        conversation_name(&page.conversation),
        Some(channel.as_str())
    );
    let message = page
        .chat_messages()
        .find(|message| message.raw_text == "batch correlation deployment")
        .expect("history contains the sent message");
    let (id, text, server_time) = confirmed_line(&sent);
    assert_eq!(text, "batch correlation deployment");
    assert_eq!(message.msgid(), Some(id));
    assert_eq!(message.server_time, server_time);
    Ok(())
}

/// reports invalid client-batch references and unknown types
#[scenario(
    requires(caps = ["batch", "draft/multiline", "standard-replies"]),
    covers = ["draft.client-batch.errors"]
)]
async fn client_batch_errors(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;

    alice
        .send_raw(papo::Line::new("BATCH").with_params(["+papo/invalid", "papo.example/unknown"]))
        .await?;
    let invalid = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(invalid.kind, StandardReplyKind::Fail);
    assert_eq!(invalid.command, "BATCH");
    assert_eq!(invalid.code, "INVALID_REFTAG");

    alice
        .send_raw(papo::Line::new("BATCH").with_params([
            "+papo-unknown",
            "papo.example/unknown",
            "#papo-missing",
        ]))
        .await?;
    let unknown = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(unknown.kind, StandardReplyKind::Fail);
    assert_eq!(unknown.command, "BATCH");
    assert_eq!(unknown.code, "UNKNOWN_TYPE");
    assert_eq!(unknown.context, ["papo-unknown", "papo.example/unknown"]);

    alice
        .send_raw(papo::Line::new("BATCH").with_params(["-papo-not-open"]))
        .await?;
    let unopened = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(unopened.kind, StandardReplyKind::Fail);
    assert_eq!(unopened.command, "BATCH");
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// an invalid real name produces a typed standard failure
#[scenario(
    requires(caps = ["standard-replies", "setname"]),
    covers = ["ircv3.standard-replies"]
)]
async fn standard_replies(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let namelen = alice.state().isupport().namelen().unwrap_or(150);
    let invalid_realname = "r".repeat(namelen + 1);
    alice
        .send_raw(papo::Line::new("SETNAME").with_params([invalid_realname]))
        .await?;
    let reply = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(reply.kind, StandardReplyKind::Fail);
    assert_eq!(reply.command, "SETNAME");
    assert_eq!(reply.code, "INVALID_REALNAME");
    assert!(reply.context.is_empty());
    assert!(!reply.description.is_empty());
    Ok(())
}
