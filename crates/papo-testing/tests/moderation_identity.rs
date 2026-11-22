//! Highest-layer moderation recipient identity regressions.

mod support {
    pub mod direct;
}

use std::time::Duration;

use papo::Reconnect;
use papo_testing::wire;
use support::direct::{connected, join, next_active_event};

const GATED_PERSON_METHODS: &[(&str, &str)] = &[
    (
        "papo::Client::action",
        "PRIVMSG alice :\u{1}ACTION waves\u{1}",
    ),
    ("papo::Client::invite", "INVITE alice #review"),
    (
        "papo::Client::kick",
        "KICK #review alice :repeated flooding",
    ),
    ("papo::Client::notice", "NOTICE alice classified"),
    ("papo::Client::privmsg", "PRIVMSG alice classified"),
    ("papo::Client::typing", "@+typing=active TAGMSG alice"),
    ("papo::Client::whois", "WHOIS alice"),
    ("papo::Kickban::nick", "KICK #review alice :ban evasion"),
    ("papo::Metadata::clear_on", "METADATA alice CLEAR"),
    ("papo::Metadata::get", "METADATA alice GET display-name"),
    ("papo::Metadata::list", "METADATA alice LIST"),
    (
        "papo::Metadata::remove_on",
        "METADATA alice SET display-name",
    ),
    (
        "papo::Metadata::set_on",
        "METADATA alice SET display-name Mallory",
    ),
    ("papo::Metadata::sync", "METADATA alice SYNC"),
    ("papo::SetModes::add_with", "MODE #review +q alice"),
    ("papo::SetModes::deop", "MODE #review -o alice"),
    ("papo::SetModes::devoice", "MODE #review -v alice"),
    ("papo::SetModes::op", "MODE #review +o alice"),
    ("papo::SetModes::remove_with", "MODE #review -a alice"),
    ("papo::SetModes::voice", "MODE #review +v alice"),
];

async fn call_person_method(
    client: &papo::Client,
    method: &str,
    alice: impl papo::AsNick + papo::AsConversation + papo::Target + Copy,
) -> Result<(), papo::Error> {
    let display_name = papo::metadata::keys::DISPLAY_NAME;
    let metadata = client.metadata();
    let modes = client.mode("#review");
    match method {
        "papo::Client::action" => client.action(alice, "waves").await.map(drop),
        "papo::Client::invite" => client.invite(alice, "#review").await,
        "papo::Client::kick" => client
            .kick("#review", alice)
            .reason("repeated flooding")
            .await
            .map(drop),
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
        "papo::Metadata::clear_on" => metadata.clear_on(alice).await.map(drop),
        "papo::Metadata::get" => metadata.get(alice, [display_name]).await.map(drop),
        "papo::Metadata::list" => metadata.list(alice).await.map(drop),
        "papo::Metadata::remove_on" => metadata.remove_on(alice, display_name).await,
        "papo::Metadata::set_on" => metadata
            .set_on(alice, display_name, "Mallory")
            .await
            .map(drop),
        "papo::Metadata::sync" => metadata.sync(alice).await.map(drop),
        "papo::SetModes::add_with" => modes.add_with('q', alice).await.map(drop),
        "papo::SetModes::remove_with" => modes.remove_with('a', alice).await.map(drop),
        "papo::SetModes::deop" => modes.deop(alice).await.map(drop),
        "papo::SetModes::devoice" => modes.devoice(alice).await.map(drop),
        "papo::SetModes::op" => modes.op(alice).await.map(drop),
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
    join(&mut events, &mut wire, ":papo!p@localhost JOIN #review").await;
    let alice = join(
        &mut events,
        &mut wire,
        ":alice!~alice@cloak.example JOIN #review",
    )
    .await;
    join(
        &mut events,
        &mut wire,
        ":mallory!~mallory@users.example JOIN #review",
    )
    .await;

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

    for (method, _) in GATED_PERSON_METHODS {
        let result = call_person_method(&client, method, &alice).await;
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

    for &(method, expected) in GATED_PERSON_METHODS {
        let (client, mut events, mut wire) = wire(
            papo::Client::builder("irc://offline.test", "papo")
                .rate_limit(papo::RateLimit::unlimited())
                .reconnect(Reconnect::never()),
        );
        wire.register(&["message-tags", "labeled-response", "draft/metadata-2"])
            .await;
        connected(&mut events).await;
        wire.server(":offline.test 005 papo CHANMODES=beI,k,l,imnst :are supported")
            .await;
        wire.server("PING :mode-rules").await;
        assert_eq!(wire.next_sent().await, "PONG mode-rules");

        let request =
            tokio::spawn(async move { call_person_method(&client, method, "alice").await });
        let mut sent = tokio::time::timeout(Duration::from_secs(1), wire.next_sent())
            .await
            .unwrap_or_else(|_| panic!("{method} did not send a command"));
        if method == "papo::Kickban::nick" {
            let (label, ban) = sent.split_once(' ').unwrap();
            assert_eq!(ban, "MODE #review +b *!*@cloak.example");
            wire.server(format!("{label} :papo!p@localhost {ban}"))
                .await;
            sent = tokio::time::timeout(Duration::from_secs(1), wire.next_sent())
                .await
                .expect("kickban did not send KICK after MODE confirmation");
        }
        let command = if sent.starts_with("@label=") {
            sent.split_once(' ').unwrap().1
        } else {
            &sent
        };
        assert_eq!(command, expected, "{method}");
        request.abort();
    }
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
    ("papo::Client::watch", "fixed nickname presence lease"),
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
            (path == "papo::Client::watch"
                || PERSON_ARGUMENTS
                    .iter()
                    .any(|argument| arguments.contains(argument)))
            .then(|| path.split('<').next().unwrap_or(path))
        })
        .collect()
}

#[tokio::test]
async fn all_public_person_methods_are_gated() {
    let classified: std::collections::BTreeSet<_> = GATED_PERSON_METHODS
        .iter()
        .chain(UNGATED_PERSON_INPUTS)
        .map(|(method, _)| *method)
        .collect();
    assert_eq!(
        classified.len(),
        GATED_PERSON_METHODS.len() + UNGATED_PERSON_INPUTS.len()
    );
    assert_eq!(person_taking_methods(), classified);
    assert_nickname_handoff_is_refused().await;
}
