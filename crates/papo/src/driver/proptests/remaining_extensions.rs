use std::time::UNIX_EPOCH;

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_bouncer_and_history_input_is_rejected_or_valid() {
    for (first, second, third) in inputs() {
        check_action(|client| {
            action(
                client
                    .bouncer()
                    .add(format!("irc://{first}:6667"))
                    .name(second.clone())
                    .nick(&third)
                    .username(second.clone())
                    .realname(third.clone())
                    .password(crate::Secret::new(first.clone())),
            )
        })
        .await;
        check_action(|client| {
            action(
                client
                    .bouncer()
                    .change(&crate::NetworkId::from(first.clone()))
                    .name(second.clone())
                    .nick(&third)
                    .username(second.clone())
                    .realname(third.clone())
                    .password(crate::Secret::new(first.clone())),
            )
        })
        .await;
        check_action(|client| {
            action(
                client
                    .bouncer()
                    .remove(&crate::NetworkId::from(first.clone())),
            )
        })
        .await;
        check_action(|client| action(client.history(first.clone()).latest().limit(second.len())))
            .await;
        check_action(|client| {
            action(
                client
                    .history(first.clone())
                    .before(crate::MessageId::from(second.clone()))
                    .limit(third.len()),
            )
        })
        .await;
        check_action(|client| {
            action(
                client
                    .history(first.clone())
                    .after(crate::MessageId::from(second.clone())),
            )
        })
        .await;
        check_action(|client| {
            action(
                client
                    .history(first.clone())
                    .around(crate::MessageId::from(second.clone())),
            )
        })
        .await;
        check_action(|client| {
            action(client.history(first.clone()).between(
                crate::MessageId::from(second.clone()),
                crate::MessageId::from(third.clone()),
            ))
        })
        .await;
        check_action(|client| action(client.mark_read(first.clone(), UNIX_EPOCH))).await;
        check_action(|client| action(client.fetch_read_marker(first.clone()))).await;
    }
    check_action(|client| action(client.bouncer().networks())).await;
    check_action(|client| action(client.recent_conversations(UNIX_EPOCH, UNIX_EPOCH))).await;
    check_action(|client| action(client.persistence().get())).await;
    check_action(|client| action(client.persistence().set(crate::PersistenceSetting::Default)))
        .await;
}
