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
    let whois = alice
        .whois(alice.nick().as_str())
        .await?
        .expect("WHOIS target exists");
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
    assert_eq!(alice.snapshot().nick(), &whois.nick);
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
    let reader = s.connect("reader").await?;
    let channel = s.channel_name();
    writer.join(&channel).await?;
    reader.join(&channel).await?;
    let sent = writer
        .privmsg(&channel, "batch correlation deployment")
        .await?;

    let page = reader.history(&channel).latest().limit(10).await?;
    assert_eq!(conversation_name(&page.conversation), Some(channel.clone()));
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
    if unknown.code != "UNKNOWN_TYPE" || unknown.context != ["papo-unknown", "papo.example/unknown"]
    {
        return Err(std::io::Error::other(format!(
            "unknown client-batch reply was {} with context {:?}, expected UNKNOWN_TYPE with the batch reference and type",
            unknown.code, unknown.context
        ))
        .into());
    }

    alice
        .send_raw(papo::Line::new("BATCH").with_params(["-papo-not-open"]))
        .await?;
    let unopened = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(unopened.kind, StandardReplyKind::Fail);
    assert_eq!(unopened.command, "BATCH");
    alice.expect_none(Event::as_disconnected).await?;
    Ok(())
}

/// rejects a raw multiline batch beyond its advertised byte limit
#[scenario(
    requires(caps = ["batch", "draft/multiline", "standard-replies"]),
    covers = ["draft.multiline.errors-limits"]
)]
async fn multiline_limit_refusal(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let state = alice.snapshot();
    let value = state
        .caps()
        .get(&papo::cap::MULTILINE)
        .and_then(Option::as_deref)
        .ok_or_else(|| std::io::Error::other("draft/multiline omitted its limits"))?;
    let max_bytes = value
        .split(',')
        .find_map(|item| item.strip_prefix("max-bytes=")?.parse::<usize>().ok())
        .ok_or_else(|| std::io::Error::other("draft/multiline omitted max-bytes"))?;
    let channel = s.channel_name();
    alice.join(&channel).await?;
    let batch = "papo-over-limit";
    alice
        .send_raw(papo::Line::new("BATCH").with_params([
            format!("+{batch}"),
            "draft/multiline".to_owned(),
            channel.clone(),
        ]))
        .await?;
    let text = "x".repeat(400);
    for _ in 0..=(max_bytes / text.len()) {
        alice
            .send_raw(
                papo::Line::new("PRIVMSG")
                    .with_tag("batch", batch)
                    .with_params([channel.clone(), text.clone()]),
            )
            .await?;
    }
    alice
        .send_raw(papo::Line::new("BATCH").with_params([format!("-{batch}")]))
        .await?;
    let refusal = alice
        .wait_for(|event| {
            event.as_standard_reply().filter(|reply| {
                reply.command == "BATCH" && reply.code == papo::fail::MULTILINE_MAX_BYTES
            })
        })
        .await?;
    assert_eq!(refusal.kind, StandardReplyKind::Fail);
    assert!(!refusal.description.is_empty());
    Ok(())
}

/// an invalid real name produces a typed standard failure
#[scenario(
    requires(caps = ["standard-replies", "setname"]),
    covers = ["ircv3.standard-replies"]
)]
async fn standard_replies(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let namelen = alice.snapshot().isupport().namelen().unwrap_or(150);
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
