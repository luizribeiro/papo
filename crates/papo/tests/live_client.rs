//! End-to-end client behavior against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Event};
use papo_test_server::{from_env, unique};

#[tokio::test(flavor = "current_thread")]
async fn registers_messages_itself_and_quits() {
    within(async {
        let server = from_env();
        let nick = unique("papo-client");
        let (client, mut events) = Client::builder(Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        ))
        .nick(support::nick(&nick))
        .username("papo")
        .realname("papo live client test")
        .connect()
        .await
        .unwrap();
        let state = client.state();
        assert!(state.is_connected() && state.is_me(&nick));

        loop {
            match events.next().await.unwrap() {
                Event::Registered(welcome) => {
                    assert_eq!(welcome.nick, nick.as_str());
                    break;
                }
                Event::Disconnected(disconnected) => {
                    panic!("disconnected while registering: {disconnected}")
                }
                _ => {}
            }
        }

        client
            .privmsg(&nick, "hello from papo's live client")
            .await
            .unwrap();
        loop {
            match events.next().await.unwrap() {
                Event::Message(message) => {
                    if message.kind() == papo::MessageKind::Privmsg
                        && message.target() == nick
                        && message.raw_text() == "hello from papo's live client"
                    {
                        break;
                    }
                }
                Event::Disconnected(disconnected) => {
                    panic!("disconnected before echo: {disconnected}")
                }
                _ => {}
            }
        }

        client.quit("live client test complete").await.unwrap();
        loop {
            match events.next().await {
                Some(Event::Disconnected(_)) => break,
                Some(_) => {}
                None => panic!("driver stopped without a disconnected event"),
            }
        }
    })
    .await;
}
