//! A replay batch papo cannot file under a conversation must still reach the application.

use std::time::Duration;

use papo::{Client, Event, RateLimit, Reconnect};
use papo_testing::wire;

#[tokio::test]
async fn a_replay_batch_with_an_unusable_target_is_not_dropped() {
    let (_client, mut events, mut wire) = wire(
        Client::builder("irc://offline.test", "papo")
            .rate_limit(RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&["batch", "draft/chathistory", "message-tags", "server-time"])
        .await;
    for row in [
        ":offline.test 005 papo STATUSMSG=@+ :are supported by this server",
        ":papo!~u@2j8y98eazaf5w.irc JOIN #review",
    ] {
        wire.server(row).await;
    }
    for (reference, target) in [("a", " @#review"), ("b", " *"), ("c", "")] {
        wire.server(format!(
            ":offline.test BATCH +{reference} chathistory{target}"
        ))
        .await;
        let audience = if reference == "a" {
            "@#review"
        } else {
            "#review"
        };
        wire.server(format!("@batch={reference};time=2026-10-15T12:00:00.000Z;msgid=6tiv6pw8a7d5mhn77ai2vm5jq2 :alice!~u@8fq2mzkq5hcsa.irc PRIVMSG {audience} :rotate the deploy key tonight")).await;
        wire.server(format!(":offline.test BATCH -{reference}"))
            .await;
    }
    wire.server(":offline.test NOTICE papo :end of scripted traffic")
        .await;

    let mut seen = Vec::new();
    loop {
        let (event, _) = tokio::time::timeout(Duration::from_secs(60), events.next())
            .await
            .expect("the scripted traffic ends with a notice")
            .expect("the driver keeps running");
        let row = match event {
            Event::ServerNotice(_) => break,
            Event::Restricted(restricted) => {
                assert_eq!(restricted.audience.symbol(), '@');
                let papo::event::RestrictedLine::Other(other) = restricted.line else {
                    panic!("expected an untyped replay");
                };
                other
            }
            Event::Other(other) if other.command() == "PRIVMSG" => other,
            Event::Message(_) | Event::AccountChanged(_) => {
                panic!("unexpected live event: {event:?}");
            }
            _ => continue,
        };
        assert!(row.is_from_history());
        assert_eq!(row.get(1), Some("rotate the deploy key tonight"));
        seen.push((
            row.tag("batch").unwrap().to_owned(),
            row.get(0).unwrap().to_owned(),
        ));
    }
    assert_eq!(
        seen.iter()
            .map(|(id, target)| (id.as_str(), target.as_str()))
            .collect::<Vec<_>>(),
        [("a", "@#review"), ("b", "#review"), ("c", "#review")]
    );
}

#[tokio::test]
async fn unsolicited_history_targets_reach_the_application() {
    let (_client, mut events, mut wire) = wire(Client::builder("irc://offline.test", "papo"));
    wire.register(&["batch", "draft/chathistory", "server-time"])
        .await;
    for row in [
        ":offline.test BATCH +targets draft/chathistory-targets",
        "@batch=targets :offline.test CHATHISTORY TARGETS #review 2026-10-15T12:00:00.000Z",
        ":offline.test BATCH -targets",
        ":offline.test NOTICE papo :end of scripted traffic",
    ] {
        wire.server(row).await;
    }
    let mut target = None;
    loop {
        let (event, _) = tokio::time::timeout(Duration::from_secs(60), events.next())
            .await
            .expect("the scripted traffic ends with a notice")
            .expect("the driver keeps running");
        match event {
            Event::ServerNotice(_) => break,
            Event::Other(other) if other.command() == "CHATHISTORY" => {
                assert!(other.is_from_history());
                assert_eq!(other.get(0), Some("TARGETS"));
                assert_eq!(other.get(2), Some("2026-10-15T12:00:00.000Z"));
                target = other.get(1).map(str::to_owned);
            }
            _ => {}
        }
    }
    assert_eq!(target.as_deref(), Some("#review"));
}
