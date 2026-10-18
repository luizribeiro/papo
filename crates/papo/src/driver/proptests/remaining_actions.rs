use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_action_builder_input_is_rejected_or_emits_valid_lines() {
    for (first, second, third) in inputs() {
        check_action({
            let (first, second) = (first.clone(), second.clone());
            move |client| action(client.set_topic(first, second))
        })
        .await;
        check_action({
            let first = first.clone();
            move |client| action(client.clear_topic(first))
        })
        .await;
        check_action({
            let first = first.clone();
            move |client| action(client.list(crate::ListFilter::default().channels([first])))
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
                let context = crate::ChannelName::from(third.clone());
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
                action(
                    client
                        .mode(first)
                        .add(added)
                        .remove(removed)
                        .add_with(added, second.clone())
                        .remove_with(removed, third.clone())
                        .ban(second.clone())
                        .unban(third.clone())
                        .op(second.clone())
                        .deop(third.clone())
                        .voice(second)
                        .devoice(third),
                )
            }
        })
        .await;
    }
}
