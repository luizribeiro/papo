//! A MONITOR update or ISUPPORT burst preserves a private sender's reference.

mod support {
    pub mod direct;
}

use papo::{Client, Event, RateLimit, Reconnect};
use papo_testing::wire;
use support::direct::{connected, enable_monitor, next_active_event};

#[tokio::test]
async fn replies_survive_monitor_sync_and_isupport() {
    for change in ["control", "watch bob", "watch alice", "ISUPPORT"] {
        let (client, mut events, mut wire) = wire(
            Client::builder("irc://offline.test", "papo")
                .rate_limit(RateLimit::unlimited())
                .reconnect(Reconnect::never()),
        );
        wire.register(&["account-tag", "message-tags"]).await;
        connected(&mut events).await;
        enable_monitor(&mut events, &mut wire, 100).await;
        wire.server(
            "@msgid=ErgoDm1;account=alice :alice!~alice@cloak-7f3a.example PRIVMSG papo :is the release out?",
        ).await;
        let Event::Message(message) = next_active_event(&mut events).await else {
            panic!("expected private message");
        };
        let _watch = match change {
            "watch bob" => {
                let watch = client.watch("bob");
                assert_eq!(wire.next_sent().await, "MONITOR + bob");
                Some(watch)
            }
            "watch alice" => {
                let watch = message.actor.watch();
                assert_eq!(wire.next_sent().await, "MONITOR + alice");
                wire.server(":offline.test 730 papo :alice!~alice@cloak-7f3a.example")
                    .await;
                while !matches!(
                    next_active_event(&mut events).await,
                    Event::MonitorOnline(_)
                ) {}
                Some(watch)
            }
            "ISUPPORT" => {
                wire.server(
                    ":offline.test 005 papo NETWORK=ErgoTest :are supported by this server",
                )
                .await;
                wire.server(":offline.test NOTICE papo :*** burst end")
                    .await;
                while !matches!(next_active_event(&mut events).await, Event::ServerNotice(_)) {}
                None
            }
            _ => None,
        };
        client
            .reply(&message, "yes, 0.1 is out")
            .await
            .unwrap_or_else(|error| {
                panic!("{change}: {error}");
            });
        assert_eq!(
            wire.next_sent().await,
            "@+reply=ErgoDm1 PRIVMSG alice :yes, 0.1 is out",
            "{change}"
        );
    }
}
