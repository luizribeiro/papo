//! Message replies against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event, Message as ChatMessage, Sasl, Source};
use papo_test_server::{create_account, from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn reply_between_two_clients_carries_the_original_msgid() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        )
        .unwrap();
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
            message.text == "!ping" && source_nick(message).as_deref() == Some(nick_a.as_str())
        })
        .await;
        assert!(!matches!(incoming.conversation, papo::Chat::Private(_)));
        let ping_msgid = (*incoming.msgid.as_ref().expect("Ergo supplies msgid")).clone();
        client_b.reply(&incoming, "pong").await.unwrap();

        let response = wait_message(&mut events_a, |message| {
            message.text == "pong" && source_nick(message).as_deref() == Some(nick_b.as_str())
        })
        .await;
        assert_eq!(response.reply_to.as_ref(), Some(&ping_msgid));

        client_b.quit("live reply test complete").await.unwrap();
        client_a.quit("live reply test complete").await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn same_account_sessions_receive_own_live_messages() {
    within(async {
        let server = from_env();
        let address = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        )
        .unwrap();
        let account = unique("papo-multiclient");
        let channel = format!("#{}", unique("papo-multiclient"));
        let password = "q7Hx-2mZp_9vLcT4wRfB";
        create_account(&server, &account, password).unwrap();
        let (client_a, _events_a) =
            account_client(address.clone(), &account, password, &channel).await;
        let (client_b, mut events_b) = account_client(address, &account, password, &channel).await;

        client_a
            .privmsg(&channel, "from this device")
            .await
            .unwrap();
        let other =
            wait_own_message(&mut events_b, |message| message.text == "from this device").await;
        assert_eq!(other.authorship, papo::Authorship::Own);

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
        )
        .unwrap();
        let channel = format!("#{}", unique("papo-react"));
        let (client_a, mut events_a) =
            client(address.clone(), &unique("papo-react-a"), &channel).await;
        let (client_b, mut events_b) = client(address, &unique("papo-react-b"), &channel).await;

        client_a.privmsg(&channel, "excellent news").await.unwrap();
        let incoming =
            wait_message(&mut events_b, |message| message.text == "excellent news").await;
        let id = incoming.msgid.as_ref().unwrap();
        client_b.react(&incoming, "🎉").await.unwrap();
        let reaction = wait_reaction(&mut events_a).await;
        assert_eq!(
            reaction.conversation,
            papo::Conversation::from(&incoming.conversation)
        );
        assert_eq!(reaction.status, incoming.status);
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
        )
        .unwrap();
        let channel = format!("#{}", unique("papo-multiline"));
        let (client_a, _events_a) =
            client(address.clone(), &unique("papo-multi-a"), &channel).await;
        let (client_b, mut events_b) = client(address, &unique("papo-multi-b"), &channel).await;
        let text = "é".repeat(500);

        let sender = client_a.clone();
        let sent_text = text.clone();
        let sent_channel = channel.clone();
        let sent = tokio::spawn(async move { sender.privmsg(&sent_channel, &sent_text).await });
        let incoming = wait_message(&mut events_b, |message| message.raw_text == text).await;
        assert_eq!(incoming.raw_text.len(), 1000);
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
        )
        .unwrap();
        let channel = format!("#{}", unique("papo-multiline-id"));
        let (client_a, mut events_a) =
            client(address.clone(), &unique("papo-multi-id-a"), &channel).await;
        let (client_b, mut events_b) = client(address, &unique("papo-multi-id-b"), &channel).await;
        assert!(client_a.state().caps().contains_key(&papo::cap::BATCH));
        assert!(client_a.state().caps().contains_key(&papo::cap::MULTILINE));

        let plain = client_a
            .privmsg(&channel, "plain message with multiline enabled")
            .await
            .unwrap();
        let plain_id = plain.msgid.expect("plain echo supplies msgid");
        assert!(plain.server_time.is_some());
        let incoming = wait_message(&mut events_b, |message| {
            message.raw_text == "plain message with multiline enabled"
        })
        .await;
        assert_eq!(incoming.msgid.as_ref(), Some(&plain_id));

        let multiline = client_a
            .privmsg(&channel, "first deployment line\nsecond deployment line")
            .await
            .unwrap();
        let multiline_id = multiline.msgid.expect("multiline echo supplies msgid");
        assert!(multiline.server_time.is_some());
        let incoming = wait_message(&mut events_b, |message| {
            message.raw_text == "first deployment line\nsecond deployment line"
        })
        .await;
        assert_eq!(incoming.msgid.as_ref(), Some(&multiline_id));

        client_b
            .privmsg(&channel, "reply through the multiline id")
            .reply_to(&multiline_id)
            .await
            .unwrap();
        let reply = wait_message(&mut events_a, |message| {
            message.raw_text == "reply through the multiline id"
        })
        .await;
        assert_eq!(reply.reply_to.as_ref(), Some(&multiline_id));

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
        .allow_insecure_auth(true)
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

async fn wait_own_message(
    events: &mut papo::Events,
    accepted: impl Fn(&ChatMessage) -> bool,
) -> ChatMessage {
    loop {
        match events.next().await {
            Some(Event::OwnMessage(message)) if accepted(&message) => return *message,
            Some(Event::Disconnected(disconnected)) => {
                panic!("disconnected while waiting for an own message: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while waiting for an own message"),
        }
    }
}

fn source_nick(message: &ChatMessage) -> Option<String> {
    match &message.actor {
        Source::User { nick, .. } => Some(nick.to_string()),
        _ => None,
    }
}
