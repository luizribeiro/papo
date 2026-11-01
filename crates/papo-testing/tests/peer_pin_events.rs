//! Highest-layer coverage for live private-event peer pins.

mod support {
    pub mod direct;
    pub mod peer_pin;
}

use papo::{Event, Reconnect};
use papo_testing::wire;
use support::{
    direct::{connected, next_active_event},
    peer_pin::join,
};

#[tokio::test]
async fn private_message_notice_and_tagmsg_expose_the_sender_pin() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("ircs://verified.example", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["account-tag", "extended-join", "message-tags"])
        .await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review papo-account :Papo",
        ":alice!~alice@cloak.example JOIN #review alice-account :Alice",
    ] {
        join(&mut events, &mut wire, line).await;
    }
    let expected = client.direct("alice").await.unwrap().peer_key().clone();

    for line in [
        "@account=alice-account :alice!~alice@cloak.example PRIVMSG papo :release ready",
        "@account=alice-account :alice!~alice@cloak.example NOTICE papo :review complete",
        "@account=alice-account;+draft/example=ready :alice!~alice@cloak.example TAGMSG papo",
    ] {
        wire.server(line).await;
        let actor = match next_active_event(&mut events).await {
            Event::Message(event) => event.actor,
            Event::Notice(event) => event.actor,
            Event::TagMsg(event) => event.actor,
            event => panic!("unexpected event: {event:?}"),
        };
        assert_eq!(actor.peer_key(), Some(&expected));
    }
}
