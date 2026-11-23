//! A message-only reference expires when the server shows a different holder of the nick.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::{Client, Error, Event, RateLimit, Reconnect};
use papo_testing::wire;
use support::direct::{connected, enable_monitor, join, next_active_event};

#[tokio::test]
#[rustfmt::skip]
async fn contradictory_arrival_expires_message_and_actor() {
    for (caps, tagged, arrival) in [
        ("message-tags", false, ":alice!~mallory@tor-exit.example JOIN #review"),
        ("message-tags account-tag", true, ":alice!~mallory@tor-exit.example JOIN #review"),
        ("message-tags account-tag extended-join", false, ":alice!~mallory@tor-exit.example JOIN #review * :Mallory"),
        ("message-tags account-tag extended-join", true, ":alice!~mallory@tor-exit.example JOIN #review mallory :Mallory"),
        ("message-tags userhost-in-names", false, ":offline.test 353 papo = #review :@alice!~mallory@tor-exit.example papo!~u@papo.irc"),
        ("message-tags", false, ":alice!~mallory@tor-exit.example PRIVMSG papo :hello?"),
        ("message-tags", false, ":offline.test 730 papo :alice!~mallory@tor-exit.example"),
    ] {
        let (client, mut events, mut wire) = wire(
            Client::builder("irc://offline.test", "papo")
                .rate_limit(RateLimit::unlimited())
                .reconnect(Reconnect::never()),
        );
        wire.register(&caps.split_whitespace().collect::<Vec<_>>()).await;
        connected(&mut events).await;
        enable_monitor(&mut events, &mut wire, 100).await;
        let account = if tagged { ";account=alice" } else { "" };
        wire.server(format!(
            "@msgid=ErgoDm1{account} :alice!~alice@cloak-7f3a.example PRIVMSG papo :is the release out?"
        )).await;
        let Event::Message(message) = next_active_event(&mut events).await else {
            panic!("expected private message");
        };
        let _watch = if arrival.contains(" 730 ") {
            let watch = client.watch("alice");
            assert_eq!(wire.next_sent().await, "MONITOR + alice");
            Some(watch)
        } else {
            let own_join = if caps.contains("extended-join") {
                ":papo!~u@papo.irc JOIN #review papo :Papo"
            } else {
                ":papo!~u@papo.irc JOIN #review"
            };
            join(&mut events, &mut wire, own_join).await;
            None
        };
        wire.server(arrival).await;
        if arrival.contains(" 353 ") {
            wire.server(":offline.test 366 papo #review :End of /NAMES list.").await;
        }
        loop {
            match next_active_event(&mut events).await {
                Event::Joined(joined) => {
                    assert_ne!(joined.actor, message.actor, "{arrival}");
                    break;
                }
                Event::Message(replacement) => {
                    assert_ne!(replacement.actor, message.actor, "{arrival}");
                    break;
                }
                Event::ChannelSynced(_) => {
                    assert_eq!(wire.next_sent().await, "MODE #review");
                    assert!(wire.next_sent().await.starts_with("WHO #review"));
                    break;
                }
                Event::MonitorOnline(_) => break,
                _ => {}
            }
        }

        let reply = client.reply(&message, "release approved").await;
        let send = client.privmsg(&message.actor, "release approved").await;
        assert!(
            matches!(reply, Err(Error::TargetGone)),
            "{arrival}: {reply:?}"
        );
        assert!(
            matches!(send, Err(Error::TargetGone)),
            "{arrival}: {send:?}"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
                .await
                .is_err(),
            "{arrival}"
        );
    }
}
