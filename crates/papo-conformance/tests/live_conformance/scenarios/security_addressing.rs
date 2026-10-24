use papo_conformance_macros::scenario;

use crate::Scenario;
use crate::scenarios::support::{conversation_name, message_text};

/// keeps a multiline direct-message reply bound to its sender across a local nick change
#[scenario(
    requires(caps = [
        "batch",
        "draft/multiline",
        "echo-message",
        "labeled-response",
        "message-tags"
    ]),
    covers = ["security.reply.multiline-dm-after-nick-change"]
)]
async fn multiline_dm_after_nick_change(s: Scenario) -> papo_testing::Result<()> {
    let mut alice = s.connect("alice").await?;
    let mut bob = s.connect("bob").await?;
    let old_nick = bob.nick();
    let new_nick = format!("r{old_nick}");
    let text = "deployment is ready\nplease approve the release";

    alice.privmsg(old_nick.as_ref(), text).await?;
    bob.set_nick(&new_nick).await?;
    let message = bob.wait_for(|event| message_text(event, text)).await?;
    assert_eq!(
        conversation_name(&message.conversation),
        Some(alice.nick().as_ref())
    );
    assert_eq!(message.return_address(), alice.nick().as_ref());

    bob.reply(&message, "release approved").await?;
    let reply = alice
        .wait_for(|event| message_text(event, "release approved"))
        .await?;
    assert_eq!(crate::source_nick(&reply.actor), Some(new_nick.as_str()));
    assert_eq!(
        conversation_name(&reply.conversation),
        Some(new_nick.as_str())
    );
    Ok(())
}

/// files a direct message replayed to an earlier nick under the other participant
#[scenario(
    requires(caps = ["batch", "draft/chathistory"]),
    covers = ["security.history.stale-dm-target"]
)]
async fn stale_dm_history_target(s: Scenario) -> papo_testing::Result<()> {
    if std::env::var("PAPO_TEST_PROFILE").as_deref() != Ok("soju-ergo") {
        return s.skip("the selected profile does not retain raw direct-message targets");
    }
    let writer = s.connect_as_account("writer").await?;
    let reader = s.connect_as_account("reader").await?;
    let old_nick = reader.nick();
    let new_nick = format!("r{old_nick}");
    let text = "can you send the incident report?";

    writer.privmsg(&old_nick, text).await?;
    reader.set_nick(&new_nick).await?;
    let page = reader
        .history(writer.nick().as_ref())
        .latest()
        .limit(10)
        .await?;
    let message = page
        .messages()
        .find(|message| message.text == text)
        .expect("direct-message history contains the stored message");
    assert_eq!(
        conversation_name(&message.conversation),
        Some(writer.nick().as_ref())
    );
    assert_eq!(message.return_address(), writer.nick().as_ref());

    reader.reply(message, "the report is attached").await?;
    Ok(())
}
