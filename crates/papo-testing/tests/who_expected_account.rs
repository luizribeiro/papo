//! Highest-layer coverage for WHO proofs of handles that carry an account but no live proof.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::{Error, Event, Reconnect};
use papo_testing::{Wire, wire};
use support::direct::{connected, next_active_event};

async fn setup() -> (papo::Client, papo::Events, Wire) {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-tag",
        "batch",
        "draft/chathistory",
        "labeled-response",
        "message-tags",
        "server-time",
    ])
    .await;
    connected(&mut events).await;
    (client, events, wire)
}

async fn history_actor(client: &papo::Client, wire: &mut Wire) -> papo::UserHandle {
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.history("#release").latest().limit(10).await }
    });
    assert_eq!(
        wire.next_sent().await,
        "@label=0 CHATHISTORY LATEST #release * 10"
    );
    for line in [
        "@label=0 :offline.test BATCH +reply labeled-response",
        "@batch=reply :offline.test BATCH +history chathistory #release",
        "@batch=history;time=2026-11-15T10:00:00.000Z;msgid=hist-alice-1;account=alice-account :alice!~alice@cloak.example PRIVMSG #release :I can review the deploy notes",
        "@batch=reply :offline.test BATCH -history",
        ":offline.test BATCH -reply",
    ] {
        wire.server(line).await;
    }
    let page = request.await.unwrap().unwrap();
    page.chat_messages().next().unwrap().actor.clone()
}

async fn inviter(events: &mut papo::Events, wire: &mut Wire) -> papo::UserHandle {
    wire.server("@account=alice-account :alice!~alice@cloak.example INVITE papo #release")
        .await;
    loop {
        if let Event::Invited(invited) = next_active_event(events).await {
            let papo::Source::User(inviter) = invited.actor else {
                panic!("INVITE from a user names a user")
            };
            return inviter;
        }
    }
}

async fn answer_who(wire: &mut Wire, account: &str) {
    let who = wire.next_sent().await;
    let (label, command) = who.split_once(' ').unwrap();
    assert_eq!(command, "WHO alice %na");
    wire.server(format!("{label} :offline.test 354 papo alice {account}"))
        .await;
    wire.server(format!(
        "{label} :offline.test 315 papo alice :End of WHO list"
    ))
    .await;
}

#[derive(Clone, Copy, Debug)]
enum Source {
    HistoryActor,
    Inviter,
}

async fn alice(
    source: Source,
    client: &papo::Client,
    events: &mut papo::Events,
    wire: &mut Wire,
) -> papo::UserHandle {
    match source {
        Source::HistoryActor => history_actor(client, wire).await,
        Source::Inviter => inviter(events, wire).await,
    }
}

#[tokio::test]
async fn an_account_carrying_handle_reaches_only_a_holder_logged_in_to_that_account() {
    for source in [Source::HistoryActor, Source::Inviter] {
        for (who_account, delivered) in [
            ("0", false),
            ("mallory-account", false),
            ("alice-account", true),
        ] {
            let (client, mut events, mut wire) = setup().await;
            let alice = alice(source, &client, &mut events, &mut wire).await;
            let sending = tokio::spawn({
                let client = client.clone();
                async move { client.privmsg(alice, "the deploy token is hunter2").await }
            });
            answer_who(&mut wire, who_account).await;
            let sent = tokio::time::timeout(Duration::from_millis(50), wire.next_sent())
                .await
                .ok();
            let result = sending.await.unwrap();
            let case = format!("{source:?} with WHO account {who_account}");
            if delivered {
                assert_eq!(
                    sent.as_deref(),
                    Some("PRIVMSG alice :the deploy token is hunter2"),
                    "{case}"
                );
                assert!(result.is_ok(), "{case}: {result:?}");
            } else {
                assert_eq!(sent, None, "{case}");
                assert!(
                    matches!(result, Err(Error::TargetGone)),
                    "{case}: {result:?}"
                );
            }
        }
    }
}
