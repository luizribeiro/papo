//! User handles against the runner-managed Ergo server.
#![cfg(sockets)]

mod support;

use support::within;

use papo::{Client, Endpoint, Error, Event, Events, PeerKeyMismatch};
use papo_test_server::{from_env, unique};

async fn connect(nick: &str, channel: &str) -> (Client, Events) {
    let server = from_env();
    let endpoint = Endpoint::plaintext(
        server.plaintext_addr().ip().to_string(),
        server.plaintext_addr().port(),
    )
    .unwrap();
    let (client, mut events) = Client::builder(endpoint, support::nick(nick))
        .autojoin([channel])
        .connect()
        .await
        .unwrap();
    wait_for(&mut events, |event| {
        event
            .as_joined()
            .filter(|joined| joined.actor.nick() == nick)
            .map(drop)
    })
    .await;
    (client, events)
}

async fn wait_for<T>(events: &mut Events, mut matches: impl FnMut(&Event) -> Option<T>) -> T {
    loop {
        match events.next().await.map(|(event, _)| event) {
            Some(Event::Disconnected(disconnected)) => panic!("disconnected: {disconnected}"),
            Some(event) => {
                if let Some(found) = matches(&event) {
                    return found;
                }
            }
            None => panic!("driver stopped while waiting for an event"),
        }
    }
}

async fn wait_message(events: &mut Events, text: &str) -> papo::Message {
    wait_for(events, |event| {
        event
            .as_message()
            .filter(|message| message.text == text)
            .cloned()
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
async fn ergo_handle_send_watch_and_open_round_trip() {
    within(async {
        let alice_nick = unique("papo-handle-alice");
        let bob_nick = unique("papo-handle-bob");
        let lobby = format!("#{}", unique("papo-handle-lobby"));
        let review = format!("#{}", unique("papo-handle-review"));
        let (alice, mut alice_events) = connect(&alice_nick, &lobby).await;
        let (bob, mut bob_events) = connect(&bob_nick, &review).await;

        let peer = bob.query(alice_nick.as_str()).unwrap();
        let watch = peer.watch();
        wait_for(&mut bob_events, |event| {
            event
                .as_monitor_online()
                .filter(|online| online.subject.nick() == alice_nick.as_str())
                .map(drop)
        })
        .await;
        assert!(watch.is_live());
        bob.privmsg(&peer, "release candidate is ready")
            .await
            .unwrap();
        let request = wait_message(&mut alice_events, "release candidate is ready").await;
        assert_eq!(request.actor.nick(), bob_nick.as_str());
        alice.privmsg(&request.actor, "approved").await.unwrap();
        wait_message(&mut bob_events, "approved").await;

        alice.join(review.as_str()).await.unwrap();
        wait_for(&mut bob_events, |event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == alice_nick.as_str())
                .map(drop)
        })
        .await;
        let key = bob.query(alice_nick.as_str()).unwrap().key();
        let opened = bob.open(&key).await.unwrap();
        assert_eq!(opened.nick(), alice_nick.as_str());
        assert_eq!(opened.key(), key);

        alice.quit("handing the nickname over").await.unwrap();
        wait_for(&mut bob_events, |event| event.as_quit().map(drop)).await;
        let (_stranger, _stranger_events) = connect(&alice_nick, &review).await;
        wait_for(&mut bob_events, |event| {
            event
                .as_joined()
                .filter(|joined| joined.actor.nick() == alice_nick.as_str())
                .map(drop)
        })
        .await;
        let Err(Error::NotSamePerson(mismatch)) = bob.open(&key).await else {
            panic!("a saved key opened the nickname's new holder");
        };
        assert_eq!(mismatch.reason(), PeerKeyMismatch::CannotVerify);
        assert!(matches!(
            bob.privmsg(&peer, "classified").await,
            Err(Error::TargetGone)
        ));
        bob.quit("handle round trip complete").await.unwrap();
    })
    .await;
}
