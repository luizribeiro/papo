use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_action_builder_input_is_rejected_or_emits_valid_lines() {
    for (first, second, third) in inputs() {
        check_action({
            let (first, second) = (first.clone(), second.clone());
            move |client| {
                action(client.set_topic(crate::Casemapping::default().channel(first), second))
            }
        })
        .await;
        check_action({
            let first = first.clone();
            move |client| action(client.clear_topic(crate::Casemapping::default().channel(first)))
        })
        .await;
        check_action({
            let first = first.clone();
            move |client| {
                action(
                    client.list(
                        crate::ListFilter::default()
                            .channels([crate::Casemapping::default().mask(first)]),
                    ),
                )
            }
        })
        .await;
        check_action({
            let first = first.clone();
            move |client| action(client.quit(first))
        })
        .await;
        check_action({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client| {
                let reply = crate::MessageId::from(third.clone());
                let context = crate::Casemapping::default().channel(third.clone());
                action(
                    client
                        .privmsg(first.clone(), second)
                        .reply_to(&reply)
                        .channel_context(&context)
                        .tag(third, first),
                )
            }
        })
        .await;
        check_action({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            let added = first.chars().next().unwrap_or('\0');
            let removed = second.chars().next().unwrap_or('\0');
            move |client| {
                let mapping = crate::Casemapping::default();
                action(
                    client
                        .mode(first)
                        .add(added)
                        .remove(removed)
                        .add_with(added, second.clone())
                        .remove_with(removed, third.clone())
                        .ban(mapping.mask(second.clone()))
                        .unban(mapping.mask(third.clone()))
                        .op(mapping.nick(second.clone()))
                        .deop(mapping.nick(third.clone()))
                        .voice(mapping.nick(second))
                        .devoice(mapping.nick(third)),
                )
            }
        })
        .await;
    }
}
