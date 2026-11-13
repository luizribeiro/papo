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

const UNSTABLE_API: &str = include_str!("../../papo/public-api-unstable.txt");

const PERSON_ARGUMENTS: &[&str] = &[
    "&papo::UserHandle",
    "impl papo::AsConversation",
    "impl papo::AsNick",
    "impl papo::Target",
];

const UNGATED_PERSON_INPUTS: &[(&str, &str)] = &[
    ("papo::Channel::member", "local state lookup"),
    ("papo::Client::builder", "the client's own nickname"),
    (
        "papo::Client::fetch_read_marker",
        "the conversation's own read marker",
    ),
    ("papo::Client::history", "the conversation's stored history"),
    (
        "papo::Client::mark_read",
        "the conversation's own read marker",
    ),
    ("papo::Client::mode", "channel or own user modes"),
    ("papo::Client::query", "local handle constructor"),
    ("papo::Client::relay", "relayed source label"),
    ("papo::Client::set_nick", "the client's own nickname"),
    ("papo::State::metadata", "local state lookup"),
    ("papo::State::presence", "local state lookup"),
    ("papo::State::read_marker", "local state lookup"),
    ("papo::State::typing", "local state lookup"),
    ("papo::State::user", "local state lookup"),
];

fn person_taking_methods() -> std::collections::BTreeSet<&'static str> {
    UNSTABLE_API
        .lines()
        .filter_map(|line| {
            let item = line.strip_prefix("pub ")?;
            let item = item.strip_prefix("async ").unwrap_or(item);
            let item = item.strip_prefix("const ").unwrap_or(item);
            let (path, signature) = item.strip_prefix("fn ")?.split_once('(')?;
            let arguments = signature.split(" -> ").next()?;
            PERSON_ARGUMENTS
                .iter()
                .any(|argument| arguments.contains(argument))
                .then(|| path.split('<').next().unwrap_or(path))
        })
        .collect()
}

#[tokio::test]
async fn all_public_person_methods_are_gated() {
    let classified: std::collections::BTreeSet<_> = GATED_PERSON_METHODS
        .iter()
        .copied()
        .chain(UNGATED_PERSON_INPUTS.iter().map(|(method, _)| *method))
        .collect();
    assert_eq!(
        classified.len(),
        GATED_PERSON_METHODS.len() + UNGATED_PERSON_INPUTS.len()
    );
    assert_eq!(person_taking_methods(), classified);
    assert_nickname_handoff_is_refused().await;
}
