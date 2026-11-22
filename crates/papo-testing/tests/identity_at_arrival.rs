//! A person named by an event is the one who sent the line, even when the event is mapped late.
//!
//! A labeled-response batch is dispatched when it closes, so every line in it has been applied
//! before papo maps the first of its events.

use papo::{Event, Events, Reconnect, UserHandle};
use papo_testing::wire;

async fn next_event(events: &mut Events) -> Event {
    match events.next().await.map(|(event, _)| event) {
        Some(Event::Disconnected(disconnected)) => {
            panic!("disconnected while waiting for an event: {disconnected}")
        }
        Some(event) => event,
        None => panic!("driver stopped while waiting for an event"),
    }
}

fn person(event: &Event) -> Option<(&'static str, UserHandle)> {
    let user = |source: &papo::Actor| source.user().cloned();
    match event {
        Event::AccountChanged(changed) => Some(("ACCOUNT", changed.actor.clone())),
        Event::AwayChanged(changed) => Some(("AWAY", changed.actor.clone())),
        Event::HostChanged(changed) => Some(("CHGHOST", changed.actor.clone())),
        Event::RealnameChanged(changed) => Some(("SETNAME", changed.actor.clone())),
        Event::ModeChanged(changed) => Some(("MODE", user(&changed.actor)?)),
        Event::TopicChanged(changed) => Some(("TOPIC", user(&changed.actor)?)),
        Event::Kicked(kicked) => Some(("KICK", user(&kicked.actor)?)),
        Event::ReadMarkerChanged(marker) => match &marker.conversation {
            papo::Conversation::Private(peer) => Some(("MARKREAD peer", peer.clone())),
            _ => None,
        },
        _ => None,
    }
}

#[tokio::test]
async fn late_mapped_events_name_the_sender_not_the_next_nick_holder() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[
        "account-notify",
        "away-notify",
        "batch",
        "chghost",
        "draft/read-marker",
        "labeled-response",
        "setname",
    ])
    .await;
    while !matches!(next_event(&mut events).await, Event::Connected(_)) {}
    let mut alice = None;
    for line in [
        ":papo-wire!~papo@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
        ":dave!~dave@users.example JOIN #review",
    ] {
        wire.server(line).await;
        loop {
            if let Event::Joined(joined) = next_event(&mut events).await {
                if joined.actor.nick() == "alice" {
                    alice = Some(joined.actor);
                }
                break;
            }
        }
    }
    let alice = alice.unwrap();

    let whois = tokio::spawn({
        let client = client.clone();
        async move { client.whois("alice").await }
    });
    let sent = wire.next_sent().await;
    let label = sent
        .strip_prefix("@label=")
        .and_then(|rest| rest.strip_suffix(" WHOIS alice"))
        .unwrap_or_else(|| panic!("expected a labeled WHOIS, got {sent}"));
    for line in [
        format!("@label={label} BATCH +whois labeled-response"),
        "@batch=whois :alice!~alice@cloak.example ACCOUNT alice".to_owned(),
        "@batch=whois :alice!~alice@cloak.example AWAY :Back in five".to_owned(),
        "@batch=whois :alice!~alice@cloak.example CHGHOST ~alice staff.example".to_owned(),
        "@batch=whois :alice!~alice@staff.example SETNAME :Alice Liddell".to_owned(),
        "@batch=whois :alice!~alice@staff.example MODE #review +v dave".to_owned(),
        "@batch=whois :alice!~alice@staff.example TOPIC #review :Release 0.1 review".to_owned(),
        "@batch=whois :alice!~alice@staff.example KICK #review dave :Flooding".to_owned(),
        "@batch=whois MARKREAD alice timestamp=2026-11-15T09:30:00.000Z".to_owned(),
        "@batch=whois :alice!~alice@staff.example QUIT :Quit: bye".to_owned(),
        "@batch=whois :mallory!~mallory@users.example NICK alice".to_owned(),
        "@batch=whois :offline.test 318 papo-wire alice :End of /WHOIS list".to_owned(),
        "BATCH -whois".to_owned(),
    ] {
        wire.server(line).await;
    }
    whois.await.unwrap().unwrap();

    let mut seen = Vec::new();
    let newcomer = loop {
        let event = next_event(&mut events).await;
        if let Event::NickChanged(changed) = event {
            break changed.actor;
        }
        let Some((line, user)) = person(&event) else {
            continue;
        };
        assert_eq!(user, alice, "{line} named someone else");
        seen.push(line);
    };
    assert_ne!(alice, newcomer);
    assert_eq!(
        seen,
        [
            "ACCOUNT",
            "AWAY",
            "CHGHOST",
            "SETNAME",
            "MODE",
            "TOPIC",
            "KICK",
            "MARKREAD peer"
        ]
    );
    assert!(matches!(
        client.whois(&alice).await,
        Err(papo::Error::TargetGone)
    ));
}
