//! Highest-layer moderation recipient identity regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, next_active_event};

async fn assert_nickname_handoff_is_refused() {
    let (client, mut events, mut wire) = wire(
        papo::Client::builder("irc://offline.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(Reconnect::never()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    for line in [
        ":papo!p@localhost JOIN #review",
        ":alice!~alice@cloak.example JOIN #review",
        ":mallory!~mallory@users.example JOIN #review",
    ] {
        wire.server(line).await;
        assert!(matches!(
            next_active_event(&mut events).await,
            papo::Event::Joined(_)
        ));
    }
    let alice = client.query("alice").unwrap();

    wire.server(":alice!~alice@cloak.example QUIT :Client Quit")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::Quit(_)
    ));
    wire.server(":mallory!~mallory@users.example NICK alice")
        .await;
    assert!(matches!(
        next_active_event(&mut events).await,
        papo::Event::NickChanged(_)
    ));

    assert!(matches!(
        client.invite(&alice, "#review").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.whois(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.metadata().list(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.mode("#review").op(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.mode("#review").halfop(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.mode("#review").voice(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    for mode in ['q', 'a'] {
        assert!(matches!(
            client.mode("#review").prefix(mode, &alice).await,
            Err(papo::Error::TargetGone)
        ));
    }
    assert!(matches!(
        client.kick("#review", &alice, "repeated flooding").await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client
            .kickban("#review")
            .nick(&alice)
            .ban("*!*@cloak.example")
            .reason("ban evasion")
            .await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.mode("#review").deop(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.mode("#review").dehalfop(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(matches!(
        client.mode("#review").devoice(&alice).await,
        Err(papo::Error::TargetGone)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), wire.next_sent())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn person_method_identity_matrix() {
    assert_nickname_handoff_is_refused().await;
}

#[tokio::test]
async fn whois_handle_refuses_handoff() {
    assert_nickname_handoff_is_refused().await;
}
