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
