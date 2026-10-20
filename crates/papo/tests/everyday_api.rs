//! Compile coverage for the everyday API exposed by the `papo` facade.

fn inspect_event(event: papo::Event) {
    match event {
        papo::Event::Message(message) => {
            let _: &papo::Conversation = &message.conversation;
            let _: Option<&papo::MessageId> = message.msgid.as_ref();
            let _: Option<&papo::ChannelName> = message.channel_context.as_ref();
        }
        papo::Event::Joined(joined) => {
            let _: &papo::ChannelName = &joined.channel;
            let _: &papo::Nick = &joined.nick;
            let _: &papo::Account = &joined.account;
        }
        _ => {}
    }
}

#[test]
fn everyday_api_is_nameable_from_the_facade() {
    let channel = papo::Casemapping::Rfc1459.channel("#papo");
    let builder: papo::Builder = papo::Client::builder("irc.example.net")
        .nick("emily")
        .autojoin(["#papo"])
        .autojoin_with_key(&channel, "correct horse battery staple");
    let (client, mut events, driver): (papo::Client, papo::Events, papo::Driver) = builder.build();

    let state: std::sync::Arc<papo::State> = client.state();
    let _: &papo::Nick = state.nick();
    let _: Option<&papo::Channel> = state.channel(&channel);
    let _: Option<&papo::User> = state.user("emily");
    let _: papo::ChannelName = state.channel_name("#papo");
    let _: papo::Nick = state.nickname("emily");

    let send: papo::SendMessage = client
        .privmsg(&channel, "hello from papo")
        .reply_to("abc123")
        .channel_context("#papo");
    let _: papo::RequestFuture<papo::Sent> = std::future::IntoFuture::into_future(send);

    let next_event = events.next();
    drop(next_event);
    let connect = papo::Client::builder("irc.example.net").connect();
    drop(connect);
    let _: fn(papo::Event) = inspect_event;
    drop(driver);
}
