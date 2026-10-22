use papo::{Event, event::StandardReplyKind};
use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::conversation_name;

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
        .messages()
        .find(|message| message.raw_text == "batch correlation deployment")
        .expect("history contains the sent message");
    assert_eq!(message.msgid.as_ref(), sent.msgid.as_ref());
    assert_eq!(message.server_time, sent.server_time);
    Ok(())
}

/// reports unknown client-batch types and invalid closing references
#[scenario(
    requires(caps = ["batch", "draft/multiline", "standard-replies"]),
    covers = ["draft.client-batch.errors"]
)]
async fn client_batch_errors(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;

    alice
        .send_raw(papo::Line::builder("BATCH").with_params([
            "+papo-unknown",
            "papo.example/unknown",
            "#papo-missing",
        ]))
        .await?;
    let unknown = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(unknown.kind, StandardReplyKind::Fail);
    assert_eq!(unknown.command, "BATCH");
    let unknown_code = unknown.code.clone();

    alice
        .send_raw(papo::Line::builder("BATCH").with_params(["-papo-not-open"]))
        .await?;
    let invalid = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(invalid.kind, StandardReplyKind::Fail);
    assert_eq!(invalid.command, "BATCH");
    let invalid_code = invalid.code.clone();
    alice.expect_none(Event::as_disconnected).await?;
    if unknown_code != "UNKNOWN_TYPE" || invalid_code != "INVALID_REFTAG" {
        return Err(std::io::Error::other(format!(
            "client-batch refusals were {unknown_code} and {invalid_code}, expected UNKNOWN_TYPE and INVALID_REFTAG"
        ))
        .into());
    }
    assert_eq!(
        unknown.context.first().map(String::as_str),
        Some("papo-unknown")
    );
    assert_eq!(
        invalid.context.first().map(String::as_str),
        Some("papo-not-open")
    );
    Ok(())
}

/// an invalid real name produces a typed standard failure
#[scenario(
    requires(caps = ["standard-replies", "setname"]),
    covers = ["ircv3.standard-replies"]
)]
async fn standard_replies(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    alice
        .send_raw(papo::Line::builder("SETNAME").with_params([""]))
        .await?;
    let reply = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(reply.kind, StandardReplyKind::Fail);
    assert_eq!(reply.command, "SETNAME");
    assert_eq!(reply.code, "INVALID_REALNAME");
    assert!(reply.context.is_empty());
    assert!(!reply.description.is_empty());
    Ok(())
}
