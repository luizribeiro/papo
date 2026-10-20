use super::*;

#[tokio::test(flavor = "current_thread")]
async fn arbitrary_extension_input_is_rejected_or_emits_valid_lines() {
    for (first, second, third) in inputs() {
        macro_rules! probe {
            ($client:ident, $a:ident, $b:ident, $c:ident => $request:expr) => {{
                let ($a, $b, $c) = (first.clone(), second.clone(), third.clone());
                check_action(move |$client| action($request)).await;
            }};
        }
        probe!(client, a, b, c => client.rename_channel(
            crate::Casemapping::default().channel(a),
            crate::Casemapping::default().channel(b),
            c
        ));
        #[cfg(feature = "unstable")]
        probe!(client, a, b, c => client.relay(
            crate::Casemapping::default().channel(a),
            crate::Casemapping::default().nick(b),
            c
        ));
        probe!(client, a, b, _c => client.monitor().add([
            crate::Casemapping::default().nick(a),
            crate::Casemapping::default().nick(b),
        ]));
        probe!(client, a, b, _c => client.monitor().remove([
            crate::Casemapping::default().nick(a),
            crate::Casemapping::default().nick(b),
        ]));
        probe!(client, a, b, c => client.create_account(
            crate::NewAccount::new(c)
                .account(crate::Casemapping::default().account(a))
                .email(b)
        ));
        probe!(client, a, b, _c => client.verify_account(
            crate::Casemapping::default().account(a),
            b
        ));
        probe!(client, a, b, _c => client.metadata().get(a, [b]));
        probe!(client, a, _b, _c => client.metadata().list(a));
        probe!(client, a, b, _c => client.metadata().set(a, b));
        probe!(client, a, _b, _c => client.metadata().remove(a));
        probe!(client, a, b, c => client.metadata().set_on(a, b, c));
        probe!(client, a, b, _c => client.metadata().remove_on(a, b));
        probe!(client, a, _b, _c => client.metadata().clear_on(a));
        probe!(client, a, b, _c => client.metadata().subscribe([a, b]));
        probe!(client, a, b, _c => client.metadata().unsubscribe([a, b]));
        probe!(client, a, _b, _c => client.metadata().sync(a));
        #[cfg(feature = "unstable")]
        probe!(client, a, b, c => client.webpush().register(a, crate::PushKeys::new(b, c)));
        #[cfg(feature = "unstable")]
        probe!(client, a, _b, _c => client.webpush().unregister(a));
    }
    check_action(|client| action(client.monitor().clear())).await;
    check_action(|client| action(client.metadata().clear())).await;
}
