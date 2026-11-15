//! The people that replies and events describe arrive as live handles.

mod support {
    pub mod direct;
}

use papo::{Events, Reconnect};
use papo_testing::{Wire, wire};
use support::direct::{connected, next_active_event};

async fn joined_review() -> (papo::Client, Events, Wire) {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    wire.server(
        ":offline.test 005 papo CHANMODES=beI,k,l,imnst MODES=4 :are supported by this server",
    )
    .await;
    for line in [
        ":papo!p@localhost JOIN #review",
        ":Alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        wire.server(line).await;
        while !matches!(next_active_event(&mut events).await, papo::Event::Joined(_)) {}
    }
    (client, events, wire)
}

async fn server_until(events: &mut Events, wire: &mut Wire, line: &str) -> papo::Event {
    wire.server(line).await;
    loop {
        match next_active_event(events).await {
            papo::Event::FeaturesChanged { .. } => {}
            event => return event,
        }
    }
}

async fn sends_op(client: &papo::Client, wire: &mut Wire, user: &papo::UserHandle) -> String {
    let request = tokio::spawn({
        let client = client.clone();
        let user = user.clone();
        async move { client.mode("#review").op(&user).await }
    });
    let sent = wire.next_sent().await;
    request.abort();
    sent
}

#[tokio::test]
async fn whois_user_follows_the_reported_person() {
    let (client, mut events, mut wire) = joined_review().await;
    let whois = tokio::spawn({
        let client = client.clone();
        async move { client.whois("alice").await }
    });
    assert_eq!(wire.next_sent().await, "WHOIS alice");
    for line in [
        ":offline.test 311 papo Alice ~alice cloak.example * :Alice Example",
        ":offline.test 319 papo Alice :#review",
        ":offline.test 312 papo Alice offline.test :ErgoTest IRC Network",
        ":offline.test 317 papo Alice 42 1760544000 :seconds idle, signon time",
        ":offline.test 318 papo Alice :End of /WHOIS list",
    ] {
        wire.server(line).await;
    }
    let user = whois.await.unwrap().unwrap().unwrap().user;
    assert_eq!(user.nick().as_str(), "Alice");
    assert_eq!(user, client.query("alice").unwrap());

    server_until(
        &mut events,
        &mut wire,
        ":Alice!~alice@cloak.example NICK alicia",
    )
    .await;
    assert_eq!(user.nick().as_str(), "alicia");
    assert_eq!(
        sends_op(&client, &mut wire, &user).await,
        "MODE #review +o alicia"
    );

    server_until(
        &mut events,
        &mut wire,
        ":alicia!~alice@cloak.example QUIT :Client Quit",
    )
    .await;
    server_until(
        &mut events,
        &mut wire,
        ":mallory!~mallory@users.example NICK alicia",
    )
    .await;
    assert!(matches!(
        client.mode("#review").op(&user).await,
        Err(papo::Error::TargetGone)
    ));
}
