//! Compile coverage for the everyday API exposed by the `papo` facade.

fn inspect_event(event: papo::Event) {
    match event {
        papo::Event::Message(message) => {
            let _: &papo::Chat = &message.conversation;
            let _: Option<&papo::MessageId> = message.msgid.as_ref();
            let _: Option<&papo::ChannelName> = message.channel_context.as_ref();
        }
        papo::Event::Joined(joined) => {
            let _: &papo::ChannelName = &joined.channel;
            let _: &papo::Source = &joined.actor;
            let _: &papo::Nick = &joined.nick;
            let _: &papo::Account = &joined.account;
        }
        _ => {}
    }
}

fn moderate_message(client: &papo::Client, event: papo::Event) {
    let papo::Event::Message(message) = event else {
        return;
    };
    let _: papo::RequestFuture<()> = client.react(&message, "👍");
    let _: papo::RequestFuture<()> = client.unreact(&message, "👍");
    let _: papo::RequestFuture<()> = client.redact(&message, "superseded");
}

fn inspect_source(source: &papo::Source) {
    let _: Option<papo::Mask> = papo::Mask::exact(source);
    let _: Option<papo::Mask> = papo::Mask::for_host(source);
    let _: Option<papo::Mask> = papo::Mask::for_user_host(source);
    if let Some(nick) = source.nick() {
        let _: papo::Mask = papo::Mask::for_nick(nick);
    }
}

#[test]
fn everyday_api_is_nameable_from_the_facade() {
    assert_eq!(
        papo::strip_formatting("\x02bold\x02 and \x0312blue"),
        "bold and blue"
    );

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
    let conversation = state.conversation("Emily").unwrap();
    assert_eq!(conversation, "emily");
    assert_eq!(conversation.to_string(), "Emily");
    let (status, conversation) = state.parse_target("#papo").unwrap();
    assert_eq!(status, None);
    assert_eq!(conversation, "#papo");

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
    let _: fn(&papo::Client, papo::Event) = moderate_message;
    let _: fn(&papo::Source) = inspect_source;
    drop(driver);
}
