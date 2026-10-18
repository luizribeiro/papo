use std::time::UNIX_EPOCH;

use papo_core::Capabilities;
use papo_proto::Isupport;

use super::*;

fn referenced_message() -> ChatMessage {
    ChatMessage::new(
        Message::privmsg("#papo", "deployment complete")
            .with_source("emily!~emily@localhost")
            .with_tag("msgid", "RfcBTzyxY120003"),
        &Isupport::default(),
        "papo-driver",
        &Capabilities::new(),
        UNIX_EPOCH,
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_context_action_text_is_rejected_or_emits_valid_lines() {
    for (_, text, reaction) in inputs() {
        check_action({
            let message = referenced_message();
            move |client| action(client.reply(&message, text))
        })
        .await;
        check_action({
            let message = referenced_message();
            move |client| action(client.react_to(&message, reaction))
        })
        .await;
    }
}
