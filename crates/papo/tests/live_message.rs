//! Message replies against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event, Message as ChatMessage, Origin, Sasl, Source};
use papo_test_server::{from_env, register_account, unique};

#[tokio::test(flavor = "current_thread")]
async fn reply_between_two_clients_carries_the_original_msgid() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let channel = format!("#{}", unique("papo-reply"));
        let nick_a = unique("papo-sender");
        let nick_b = unique("papo-replier");

        let (client_a, mut events_a) = Client::builder(address.clone())
            .nick(support::nick(&nick_a))
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_ready(&mut events_a, &nick_a, &channel).await;

        let (client_b, mut events_b) = Client::builder(address)
            .nick(support::nick(&nick_b))
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_ready(&mut events_b, &nick_b, &channel).await;

        client_a.privmsg(&channel, "!ping").await.unwrap();
        let incoming = wait_message(&mut events_b, |message| {
            message.text() == "!ping" && source_nick(message).as_deref() == Some(nick_a.as_str())
        })
        .await;
        assert!(incoming.is_live());
        assert!(!incoming.is_private());
        let ping_msgid = incoming.msgid().expect("Ergo supplies msgid").to_owned();
        client_b.reply(&incoming, "pong").await.unwrap();

        let response = wait_message(&mut events_a, |message| {
            message.text() == "pong" && source_nick(message).as_deref() == Some(nick_b.as_str())
        })
        .await;
        assert_eq!(response.reply_to(), Some(ping_msgid.as_str()));

        client_b.quit("live reply test complete").await.unwrap();
        client_a.quit("live reply test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn same_account_sessions_distinguish_echoes() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let account = unique("papo-multiclient");
        let channel = format!("#{}", unique("papo-multiclient"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        register_account(&server, &account, password).unwrap();
        let (client_a, mut events_a) =
            account_client(address.clone(), &account, password, &channel).await;
        let (client_b, mut events_b) = account_client(address, &account, password, &channel).await;

        client_a
            .privmsg(&channel, "from this device")
            .await
            .unwrap();
        let echo = wait_message(&mut events_a, |message| {
            message.text() == "from this device" && message.origin() == Origin::Echo
        })
        .await;
        let other = wait_message(&mut events_b, |message| {
            message.text() == "from this device" && message.origin() == Origin::OtherSession
        })
        .await;
        assert_eq!(echo.origin(), Origin::Echo);
        assert_eq!(other.origin(), Origin::OtherSession);
        assert!(!other.is_live());

        client_b.quit("multiclient test complete").await.unwrap();
        client_a.quit("multiclient test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn reactions_reach_another_client_with_reply_context() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let channel = format!("#{}", unique("papo-react"));
        let (client_a, mut events_a) =
            client(address.clone(), &unique("papo-react-a"), &channel).await;
        let (client_b, mut events_b) = client(address, &unique("papo-react-b"), &channel).await;

        client_a.privmsg(&channel, "excellent news").await.unwrap();
        let incoming =
            wait_message(&mut events_b, |message| message.text() == "excellent news").await;
        let id = incoming.id().unwrap();
        client_b.react_to(&incoming, "🎉").await.unwrap();
        let reaction = wait_reaction(&mut events_a).await;
        assert_eq!(reaction.target, incoming.target);
        assert_eq!(&reaction.message, id);

        client_b.quit("reaction test complete").await.unwrap();
        client_a.quit("reaction test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn multiline_delivers_a_thousand_byte_message_whole() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let channel = format!("#{}", unique("papo-multiline"));
        let (client_a, _events_a) =
            client(address.clone(), &unique("papo-multi-a"), &channel).await;
        let (client_b, mut events_b) = client(address, &unique("papo-multi-b"), &channel).await;
        let text = "é".repeat(500);

        let sender = client_a.clone();
        let sent_text = text.clone();
        let sent_channel = channel.clone();
        let sent = tokio::spawn(async move { sender.privmsg(&sent_channel, &sent_text).await });
        let incoming = wait_message(&mut events_b, |message| message.raw_text() == text).await;
        assert_eq!(incoming.raw_text().len(), 1000);
        sent.await.unwrap().unwrap();

        client_b.quit("multiline test complete").await.unwrap();
        client_a.quit("multiline test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn multiline_capability_preserves_plain_and_batched_message_ids() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        );
        let channel = format!("#{}", unique("papo-multiline-id"));
        let (client_a, mut events_a) =
            client(address.clone(), &unique("papo-multi-id-a"), &channel).await;
        let (client_b, mut events_b) = client(address, &unique("papo-multi-id-b"), &channel).await;
        assert!(client_a.state().has_cap("batch"));
        assert!(client_a.state().has_cap("draft/multiline"));

        let plain = client_a
            .privmsg(&channel, "plain message with multiline enabled")
            .await
            .unwrap();
        let plain_id = plain.id.expect("plain echo supplies msgid");
        assert!(plain.time.is_some());
        let incoming = wait_message(&mut events_b, |message| {
            message.raw_text() == "plain message with multiline enabled"
        })
        .await;
        assert_eq!(incoming.id(), Some(&plain_id));

        let multiline = client_a
            .privmsg(&channel, "first deployment line\nsecond deployment line")
            .await
            .unwrap();
        let multiline_id = multiline.id.expect("multiline echo supplies msgid");
        assert!(multiline.time.is_some());
        let incoming = wait_message(&mut events_b, |message| {
            message.raw_text() == "first deployment line\nsecond deployment line"
        })
        .await;
        assert_eq!(incoming.id(), Some(&multiline_id));

        client_b
            .privmsg(&channel, "reply through the multiline id")
            .reply_to(&multiline_id)
            .await
            .unwrap();
        let reply = wait_message(&mut events_a, |message| {
            message.raw_text() == "reply through the multiline id"
        })
        .await;
        assert_eq!(reply.reply_to(), Some(multiline_id.as_ref()));

        client_b.quit("multiline id test complete").await.unwrap();
        client_a.quit("multiline id test complete").await.unwrap();
    })
    .await;
}

async fn client(address: Endpoint, nick: &str, channel: &str) -> (Client, papo::Events) {
    let (client, mut events) = Client::builder(address)
        .nick(support::nick(nick))
        .autojoin([channel])
        .connect()
        .await
        .unwrap();
    wait_ready(&mut events, nick, channel).await;
    (client, events)
}

async fn account_client(
    address: Endpoint,
    account: &str,
    password: &str,
    channel: &str,
) -> (Client, papo::Events) {
    let (client, mut events) = Client::builder(address)
        .nick(support::nick(account))
        .sasl(Sasl::plain(account, password))
        .autojoin([channel])
        .connect()
        .await
        .unwrap();
    wait_ready(&mut events, account, channel).await;
    (client, events)
}

async fn wait_reaction(events: &mut papo::Events) -> papo::event::Reacted {
    loop {
        match events.next().await {
            Some(Event::Reacted(reaction)) => return reaction,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for a reaction"),
        }
    }
}

async fn wait_ready(events: &mut papo::Events, nick: &str, channel: &str) {
    let mut registered = false;
    loop {
        match events.next().await {
            Some(Event::Connected(welcome)) => registered = welcome.nick == nick,
            Some(Event::Joined(papo::event::Joined {
                channel: joined,
                nick: joined_nick,
                ..
            })) if registered && joined == channel && joined_nick == nick => return,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected while joining: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}

async fn wait_message(
    events: &mut papo::Events,
    accepted: impl Fn(&ChatMessage) -> bool,
) -> ChatMessage {
    loop {
        match events.next().await {
            Some(Event::Message(message)) if accepted(&message) => return *message,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected while waiting for a message: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for a message"),
        }
    }
}

fn source_nick(message: &ChatMessage) -> Option<String> {
    match message.source() {
        Some(Source::User { nick, .. }) => Some(nick.into_string()),
        _ => None,
    }
}
