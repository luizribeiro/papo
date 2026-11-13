//! Highest-layer moderation recipient identity regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, next_active_event};

const GATED_PERSON_METHODS: &[&str] = &[
    "papo::Client::action",
    "papo::Client::invite",
    "papo::Client::kick",
    "papo::Client::notice",
    "papo::Client::privmsg",
    "papo::Client::typing",
    "papo::Client::whois",
    "papo::Kickban::nick",
    "papo::Metadata::clear_on",
    "papo::Metadata::get",
    "papo::Metadata::list",
    "papo::Metadata::remove_on",
    "papo::Metadata::set_on",
    "papo::Metadata::sync",
    "papo::SetModes::dehalfop",
    "papo::SetModes::deop",
    "papo::SetModes::devoice",
    "papo::SetModes::halfop",
    "papo::SetModes::op",
    "papo::SetModes::prefix",
    "papo::SetModes::voice",
];

async fn call_with_handed_off_person(
    client: &papo::Client,
    method: &str,
    alice: &papo::UserHandle,
) -> Result<(), papo::Error> {
    let display_name = papo::metadata::keys::DISPLAY_NAME;
    let metadata = client.metadata();
    let modes = client.mode("#review");
    match method {
        "papo::Client::action" => client.action(alice, "waves").await.map(drop),
        "papo::Client::invite" => client.invite(alice, "#review").await,
        "papo::Client::kick" => client.kick("#review", alice, "repeated flooding").await,
        "papo::Client::notice" => client.notice(alice, "classified").await.map(drop),
        "papo::Client::privmsg" => client.privmsg(alice, "classified").await.map(drop),
        "papo::Client::typing" => client.typing(alice, papo::Typing::Active).await,
        "papo::Client::whois" => client.whois(alice).await.map(drop),
        "papo::Kickban::nick" => client
            .kickban("#review")
            .nick(alice)
            .ban("*!*@cloak.example")
            .reason("ban evasion")
            .await
            .map(drop),
        "papo::Metadata::clear_on" => metadata.clear_on(alice).await,
        "papo::Metadata::get" => metadata.get(alice, [display_name]).await.map(drop),
        "papo::Metadata::list" => metadata.list(alice).await.map(drop),
        "papo::Metadata::remove_on" => metadata.remove_on(alice, display_name).await,
        "papo::Metadata::set_on" => metadata.set_on(alice, display_name, "Mallory").await,
        "papo::Metadata::sync" => metadata.sync(alice).await,
        "papo::SetModes::dehalfop" => modes.dehalfop(alice).await.map(drop),
        "papo::SetModes::deop" => modes.deop(alice).await.map(drop),
        "papo::SetModes::devoice" => modes.devoice(alice).await.map(drop),
        "papo::SetModes::halfop" => modes.halfop(alice).await.map(drop),
        "papo::SetModes::op" => modes.op(alice).await.map(drop),
        "papo::SetModes::prefix" => {
            let owner = modes.prefix('q', alice).await;
            let admin = client.mode("#review").prefix('a', alice).await;
            match (owner, admin) {
                (Err(papo::Error::TargetGone), Err(papo::Error::TargetGone)) => {
                    Err(papo::Error::TargetGone)
                }
                _ => Ok(()),
            }
        }
        "papo::SetModes::voice" => modes.voice(alice).await.map(drop),
        other => panic!("{other} has no nickname-handoff check"),
    }
}

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

    for method in GATED_PERSON_METHODS {
        let result = call_with_handed_off_person(&client, method, &alice).await;
        assert!(
            matches!(result, Err(papo::Error::TargetGone)),
            "{method} did not refuse the handoff: {result:?}"
        );
    }
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
