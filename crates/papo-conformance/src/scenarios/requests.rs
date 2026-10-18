use papo::{Event, Origin, event::StandardReplyKind};
use papo_conformance_macros::scenario;

use crate::Scenario;

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
    assert!(whois.user.as_deref().is_some_and(|user| !user.is_empty()));
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
    let page = reader.history(&channel).latest(10).await?;
    assert_eq!(page.target.as_ref(), channel);
    let message = page
        .messages()
        .find(|message| message.raw_text() == "batch correlation deployment")
        .expect("history contains the sent message");
    assert_eq!(message.id(), sent.id.as_ref());
    assert_eq!(message.server_time, sent.time);
    assert_eq!(message.origin(), Origin::History);
    Ok(())
}

/// an invalid real name produces a typed standard failure
#[scenario(
    requires(caps = ["standard-replies", "setname"]),
    covers = ["ircv3.standard-replies"]
)]
async fn standard_replies(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    alice.send_command("SETNAME", [""]).await?;
    let reply = alice.wait_for(Event::as_standard_reply).await?;
    assert_eq!(reply.kind, StandardReplyKind::Fail);
    assert_eq!(reply.command, "SETNAME");
    assert_eq!(reply.code, "INVALID_REALNAME");
    assert!(reply.context.is_empty());
    assert!(!reply.description.is_empty());
    Ok(())
}
