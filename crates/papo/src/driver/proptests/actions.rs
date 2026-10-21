use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_core_action_input_is_rejected_or_emits_valid_lines() {
    for (first, second, third) in inputs() {
        check_action({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client| action(client.send_raw(crate::Line::new(first, [second, third])))
        })
        .await;
        check_action({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client| action(client.request_raw(crate::Line::new(first, [second, third])))
        })
        .await;

        macro_rules! check {
            ($call:expr) => {
                let call = $call;
                check_action(move |client| action(call(&client))).await
            };
        }
        check!({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client: &crate::Client| client.react((first, second), third)
        });
        check!({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client: &crate::Client| client.unreact((first, second), third)
        });
        check!({
            let first = first.clone();
            move |client: &crate::Client| client.typing(first, Typing::Active)
        });
        check!({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client: &crate::Client| client.redact((first, second), third)
        });
        check!({
            let first = first.clone();
            move |client: &crate::Client| client.whois(crate::Casemapping::default().nick(first))
        });
        check!({
            let first = first.clone();
            move |client: &crate::Client| client.who(crate::Casemapping::default().mask(first))
        });
        check!({
            let (first, third) = (first.clone(), third.clone());
            move |client: &crate::Client| {
                client
                    .join(crate::Casemapping::default().channel(first))
                    .key(third)
            }
        });
        check!({
            let (first, second) = (first.clone(), second.clone());
            move |client: &crate::Client| {
                client.part(crate::Casemapping::default().channel(first), second)
            }
        });
        check!({
            let (first, second, third) = (first.clone(), second.clone(), third.clone());
            move |client: &crate::Client| {
                let mapping = crate::Casemapping::default();
                client.kick(mapping.channel(first), mapping.nick(second), third)
            }
        });
        check!({
            let (first, second) = (first.clone(), second.clone());
            move |client: &crate::Client| {
                let mapping = crate::Casemapping::default();
                client.invite(mapping.channel(first), mapping.nick(second))
            }
        });
        check!({
            let first = first.clone();
            move |client: &crate::Client| client.set_nick(crate::Casemapping::default().nick(first))
        });
        check!({
            let first = first.clone();
            move |client: &crate::Client| client.set_away(first)
        });
        check!({
            let first = first.clone();
            move |client: &crate::Client| client.set_realname(first)
        });
        check!({
            let (first, second) = (first.clone(), second.clone());
            move |client: &crate::Client| client.oper(first, second)
        });
    }
}
