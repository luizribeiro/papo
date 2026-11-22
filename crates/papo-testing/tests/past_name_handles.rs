//! A person a stored or archived line names is a past name: a send through it must not reach
//! whoever holds that nickname now.

mod support {
    pub mod direct;
}

use std::{sync::Arc, time::Duration};

use papo::{Event, Reconnect, UserHandle};
use papo_testing::{Wire, wire};
use support::direct::connected;

/// Reads client lines up to `expected`, skipping the queries papo sends after a join, and
/// returns the tags to answer it with.
async fn sent_after_join_queries(wire: &mut Wire, expected: &str) -> String {
    loop {
        let line = wire.next_sent().await;
        let (tags, command) = match line.split_once(' ') {
            Some((tags, command)) if tags.starts_with('@') => (format!("{tags} "), command),
            _ => (String::new(), line.as_str()),
        };
        if command == expected {
            return tags;
        }
        assert!(!command.starts_with("PRIVMSG"), "sent {line}");
    }
}

async fn joined_with_topic(events: &mut papo::Events, wire: &mut Wire) -> Arc<papo::State> {
    for line in [
        ":papo!~papo@localhost JOIN #release",
        ":ergo.test 332 papo #release :release freeze, ask carol for the token",
        ":ergo.test 333 papo #release carol!~carol@cloak.example 1700000000",
        ":ergo.test 353 papo = #release :@papo",
        ":ergo.test 366 papo #release :End of /NAMES list.",
    ] {
        wire.server(line).await;
    }
    loop {
        let (_, state) = events.next().await.expect("driver stopped");
        if state
            .channel("#release")
            .ok()
            .flatten()
            .is_some_and(|channel| channel.topic_set_by().is_some())
        {
            return state;
        }
    }
}

async fn ban_setter(client: &papo::Client, wire: &mut Wire) -> UserHandle {
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.list_modes("#release", papo::ListMode::Ban).await }
    });
    let label = sent_after_join_queries(wire, "MODE #release b").await;
    wire.server(format!("{label}:ergo.test BATCH +list labeled-response"))
        .await;
    for line in [
        "@batch=list :ergo.test 367 papo #release *!*@spam.example dave!~dave@cloak.example 1700000000",
        "@batch=list :ergo.test 368 papo #release :End of channel ban list",
        ":ergo.test BATCH -list",
    ] {
        wire.server(line).await;
    }
    let entries = request.await.unwrap().unwrap();
    let setter = entries[0].actor().and_then(papo::Source::user);
    setter.cloned().unwrap()
}

async fn archived_actors(
    client: &papo::Client,
    wire: &mut Wire,
) -> Vec<(&'static str, UserHandle)> {
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.history("#release").latest().limit(10).await }
    });
    let label = sent_after_join_queries(wire, "CHATHISTORY LATEST #release * 10").await;
    wire.server(format!("{label}:ergo.test BATCH +reply labeled-response"))
        .await;
    for line in [
        "@batch=reply :ergo.test BATCH +history chathistory #release",
        "@batch=history;time=2026-11-14T09:00:00.000Z;msgid=h-join :alice!~alice@cloak.example JOIN #release",
        "@batch=history;time=2026-11-14T09:01:00.000Z;msgid=h-topic :alice!~alice@cloak.example TOPIC #release :ask alice for the token",
        "@batch=history;time=2026-11-14T09:02:00.000Z;msgid=h-quit :alice!~alice@cloak.example QUIT :Quit: bye",
        "@batch=reply :ergo.test BATCH -history",
        ":ergo.test BATCH -reply",
    ] {
        wire.server(line).await;
    }
    let page = request.await.unwrap().unwrap();
    page.items
        .iter()
        .filter_map(|item| match item {
            Event::Joined(joined) => Some(("archived JOIN", joined.actor.clone())),
            Event::TopicChanged(topic) => Some(("archived TOPIC", topic.actor.user()?.clone())),
            Event::Quit(quit) => Some(("archived QUIT", quit.actor.clone())),
            _ => None,
        })
        .collect()
}

async fn send_is_refused(client: &papo::Client, wire: &mut Wire, kind: &str, target: UserHandle) {
    let result = tokio::select! {
        result = client.privmsg(target, "the deploy token is hunter2") => result,
        line = wire.next_sent() => panic!("{kind}: sent {line}"),
    };
    assert!(
        matches!(result, Err(papo::Error::TargetGone)),
        "{kind}: {result:?}"
    );
    let sent = tokio::time::timeout(Duration::from_millis(50), wire.next_sent()).await;
    assert!(sent.is_err(), "{kind}: sent {sent:?}");
}

#[tokio::test]
async fn past_names_never_reach_the_current_holder() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "batch",
        "draft/chathistory",
        "labeled-response",
        "message-tags",
        "server-time",
    ])
    .await;
    connected(&mut events).await;

    let state = joined_with_topic(&mut events, &mut wire).await;
    let channel = state.channel("#release").unwrap().unwrap();
    let topic_setter = channel.topic_set_by().and_then(papo::Source::user);
    let mut targets = vec![("333 topic setter", topic_setter.cloned().unwrap())];
    targets.push(("367 ban setter", ban_setter(&client, &mut wire).await));
    targets.extend(archived_actors(&client, &mut wire).await);
    let kinds: Vec<_> = targets.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds.join(", "),
        "333 topic setter, 367 ban setter, archived JOIN, archived TOPIC, archived QUIT"
    );

    for (nick, line) in [
        ("carol", ":carol!~eve@guest.example JOIN #release"),
        ("dave", ":dave!~mallory@visitor.example JOIN #release"),
        ("alice", ":alice!~pat@cloak.example JOIN #release"),
    ] {
        wire.server(line).await;
        loop {
            if let Event::Joined(joined) = support::direct::next_active_event(&mut events).await
                && joined.actor == nick
            {
                break;
            }
        }
    }

    for (kind, target) in targets {
        send_is_refused(&client, &mut wire, kind, target).await;
    }
}
