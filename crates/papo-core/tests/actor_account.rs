//! Transcript coverage for the account evidence delivered with IRC lines.

use std::time::Instant;

use papo_core::{Config, Event, Party, Session, cap::ACCOUNT_TAG};
use papo_proto::Source;

#[test]
fn account_tags_and_source_roles_survive_dispatch() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo"));
    session.handle_connected(now);
    assert!(session.poll_transmit().is_some());
    for line in [
        ":ergo.test CAP * LS :account-tag echo-message",
        ":ergo.test CAP * ACK :account-tag echo-message",
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        ":ergo.test 005 papo CASEMAPPING=ascii CHANTYPES=# :are supported",
        ":ergo.test 422 papo :MOTD File is missing",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    while session.poll_event().is_some() {}
    for (line, account, party, user) in [
        (
            "@account=alice :alice!~alice@cloak.example PRIVMSG papo :release ready",
            Some("alice"),
            Party::Other,
            true,
        ),
        (
            "@+account=alice :bob!~bob@cloak.example PRIVMSG papo :release ready",
            None,
            Party::Other,
            true,
        ),
        (
            ":papo!~papo@localhost PRIVMSG bob :thanks for the update",
            None,
            Party::Me,
            true,
        ),
        (
            ":ergo.test NOTICE papo :Maintenance in five minutes",
            None,
            Party::Other,
            false,
        ),
        (
            ":NickServ!NickServ@localhost NOTICE papo :You're not logged into an account",
            None,
            Party::Other,
            true,
        ),
    ] {
        assert!(session.caps().contains_key(&ACCOUNT_TAG));
        session.handle_line(now, line.as_bytes());
        let Some(Event::Line(received)) = session.poll_event() else {
            panic!("expected a line event for {line}");
        };
        assert_eq!(
            received.as_ref().tag("account").as_deref(),
            account,
            "{line}"
        );
        assert_eq!(received.actor_party(), party, "{line}");
        assert_eq!(
            matches!(received.as_ref().source(), Some(Source::User { .. })),
            user,
            "{line}"
        );
        assert!(session.poll_event().is_none());
    }
    session.handle_line(now, b":ergo.test CAP papo DEL :account-tag");
    assert!(!session.caps().contains_key(&ACCOUNT_TAG));
    session.handle_line(now, b":ergo.test BATCH +h chathistory #review");
    session.handle_line(
        now,
        b"@batch=h;time=2026-10-15T12:00:00.000Z :alice!~alice@cloak.example PRIVMSG #review :earlier update",
    );
    session.handle_line(now, b":ergo.test BATCH -h");
    let items = std::iter::from_fn(|| session.poll_event())
        .find_map(|event| match event {
            Event::History { items, .. } => Some(items),
            _ => None,
        })
        .expect("history page");
    let [item] = items.as_slice() else {
        panic!("expected one history item");
    };
    let item = item.as_message().expect("history message");
    assert_eq!(item.actor_party, Party::Unknown);
    assert!(item.line.as_ref().tag("account").is_none());
}

mod support;

use support::Transcript;

#[test]
fn ergo_account_login_is_the_event_and_tracked_account() {
    account_transcript()
        .server(":papo!~u@2j8y98eazaf5w.irc JOIN #audit * :Papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server("@msgid=kq39fjtm8izshyfc99b32wwzz6;time=2026-11-23T10:37:07.869Z :actor!~u@2j8y98eazaf5w.irc JOIN #audit * :actor real")
        .event(|event| matches!(event, Event::Joined { .. }))
        .advance(std::time::Duration::from_secs(2))
        .server("@time=2026-11-23T10:37:09.874Z :actor!~u@2j8y98eazaf5w.irc ACCOUNT actor")
        .event(|event| matches!(event, Event::AccountChanged { envelope, account, .. }
            if envelope.account() == Some("actor")
                && matches!(account, papo_core::AccountStatus::LoggedIn(name) if name == "actor")))
        .inspect(assert_tracks_actor_account)
        .server("@account=actor :actor!~u@2j8y98eazaf5w.irc ACCOUNT *")
        .event(|event| matches!(event, Event::AccountChanged { envelope, account: papo_core::AccountStatus::Unknown, .. }
            if envelope.account().is_none() && !envelope.__account_known()))
        .inspect(|session| assert_eq!(
            session.state().user("actor").unwrap().account(),
            &papo_core::AccountStatus::Unknown
        ))
        .no_client();
}

#[test]
fn untagged_extended_join_is_the_event_and_tracked_account() {
    account_transcript()
        .server(":papo!~u@2j8y98eazaf5w.irc JOIN #audit * :Papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server("@time=2026-11-23T10:37:23.889Z :actor!~u@2j8y98eazaf5w.irc JOIN #audit actor :New Real")
        .event(|event| matches!(event, Event::Joined { envelope, .. }
            if envelope.account() == Some("actor")))
        .inspect(assert_tracks_actor_account)
        .server(":papo!~u@2j8y98eazaf5w.irc JOIN #triage * :Papo")
        .event(|event| matches!(event, Event::Joined { .. }))
        .server("@account=previous :actor!~u@2j8y98eazaf5w.irc JOIN #triage actor :New Real")
        .event(|event| matches!(event, Event::Joined { envelope, .. }
            if envelope.account().is_none() && !envelope.__account_known()))
        .inspect(|session| assert_eq!(
            session.state().user("actor").unwrap().account(),
            &papo_core::AccountStatus::Unknown
        ))
        .no_client();
}

fn account_transcript() -> Transcript {
    Transcript::new(Config::default())
        .connect_default()
        .server(":ergo.test CAP * LS :account-notify account-tag extended-join")
        .client("CAP REQ :account-tag extended-join account-notify")
        .server(":ergo.test CAP * ACK :account-notify account-tag extended-join")
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
}

fn assert_tracks_actor_account(session: &Session) {
    assert!(matches!(
        session.state().user("actor").unwrap().account(),
        papo_core::AccountStatus::LoggedIn(name) if name == "actor"
    ));
}
