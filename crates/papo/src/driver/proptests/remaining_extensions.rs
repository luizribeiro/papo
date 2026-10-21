use std::time::UNIX_EPOCH;

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_bouncer_and_history_input_is_rejected_or_valid() {
    for (first, second, third) in inputs() {
        let config = || {
            crate::NetworkConfig::new(first.clone())
                .name(second.clone())
                .port(u16::try_from(first.len() % 65_536).unwrap())
                .tls(first.len() % 2 == 0)
                .nickname(crate::Casemapping::default().nick(third.clone()))
                .username(second.clone())
                .realname(third.clone())
                .password(first.clone())
        };
        check_action(|client| action(client.bouncer().add(config()))).await;
        check_action(|client| {
            action(
                client
                    .bouncer()
                    .change(&crate::NetworkId::from(first.clone()), config()),
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
                    .before(second.clone())
                    .limit(third.len()),
            )
        })
        .await;
        check_action(|client| action(client.history(first.clone()).after(second.clone()))).await;
        check_action(|client| action(client.history(first.clone()).around(second.clone()))).await;
        check_action(|client| {
            action(
                client
                    .history(first.clone())
                    .between(second.clone(), third.clone()),
            )
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
