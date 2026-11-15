//! Transcript coverage for people papo knows only from their private lines, who can leave and be
//! replaced under the same nick without papo seeing it.

use std::time::Instant;

use papo_core::{Account, Config, Event, RecipientStatus, ReplyToken, Session};

const ALICE_ADMIN: &str =
    "@account=alice;msgid=4kd8y2mxa1 :alice!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :!ping";

fn server(session: &mut Session, line: &str) {
    session.handle_line(Instant::now(), line.as_bytes());
}

fn drain(session: &mut Session) -> Vec<Event> {
    while session.poll_transmit().is_some() {}
    std::iter::from_fn(|| session.poll_event()).collect()
}

fn registered(caps: &str) -> Session {
    let mut session = Session::new(Config::default().nick("papo-admin"));
    session.handle_connected(Instant::now());
    drain(&mut session);
    server(&mut session, &format!(":ergo.test CAP * LS :{caps}"));
    drain(&mut session);
    server(&mut session, &format!(":ergo.test CAP * ACK :{caps}"));
    for line in [
        ":ergo.test 001 papo-admin :Welcome to the ErgoTest IRC Network papo-admin",
        ":ergo.test 005 papo-admin CASEMAPPING=rfc1459 CHANTYPES=# :are supported by this server",
        ":ergo.test 422 papo-admin :MOTD File is missing",
    ] {
        server(&mut session, line);
    }
    drain(&mut session);
    session
}

fn account(session: &Session, nick: &str) -> Option<Account> {
    session
        .state()
        .user(nick)
        .map(|user| user.account().clone())
}

fn logged_in(session: &Session, name: &str) -> Account {
    Account::LoggedIn(session.state().casemapping().account(name))
}

fn token(session: &Session, nick: &str) -> ReplyToken {
    session.state().__reply_token_for(nick)
}

#[test]
fn an_unseen_nick_handoff_does_not_inherit_the_admins_identity_or_account() {
    let mut session = registered("account-tag message-tags");
    server(&mut session, ALICE_ADMIN);
    drain(&mut session);
    let admin = token(&session, "alice");
    let alice = logged_in(&session, "alice");
    assert_eq!(account(&session, "alice"), Some(alice.clone()));
    let guard = session
        .state()
        .__capture_recipient(session.state().casemapping().nick("alice"), alice)
        .unwrap();

    server(
        &mut session,
        "@msgid=9fq3nb7c2e :alice!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :!op alice",
    );
    let events = drain(&mut session);

    assert_eq!(account(&session, "alice"), Some(Account::LoggedOut));
    assert_ne!(token(&session, "alice"), admin);
    assert_eq!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Gone
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::AccountChanged { .. })),
        "a new person is not an account change: {events:?}"
    );
}

struct Handoff {
    name: &'static str,
    caps: &'static str,
    first: &'static str,
    second: &'static str,
    same_person: bool,
    account: Option<&'static str>,
}

#[test]
fn a_person_papo_cannot_follow_stays_the_same_only_while_every_fact_matches() {
    for case in [
        Handoff {
            name: "same host, still logged in",
            caps: "account-tag message-tags",
            first: ALICE_ADMIN,
            second: "@account=alice :alice!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :!status",
            same_person: true,
            account: Some("alice"),
        },
        Handoff {
            name: "same host, now without the account tag",
            caps: "account-tag message-tags",
            first: ALICE_ADMIN,
            second: ":alice!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :!op alice",
            same_person: false,
            account: None,
        },
        Handoff {
            name: "another host, same account",
            caps: "account-tag message-tags",
            first: ALICE_ADMIN,
            second: "@account=alice :alice!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :!status",
            same_person: false,
            account: Some("alice"),
        },
        Handoff {
            name: "another username",
            caps: "account-tag message-tags",
            first: ALICE_ADMIN,
            second: "@account=alice :alice!mallory@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :!status",
            same_person: false,
            account: Some("alice"),
        },
        Handoff {
            name: "another account",
            caps: "account-tag message-tags",
            first: ALICE_ADMIN,
            second: "@account=mallory :alice!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :!status",
            same_person: false,
            account: Some("mallory"),
        },
        Handoff {
            name: "logged out, then logged in",
            caps: "account-tag message-tags",
            first: ":guest!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :hello",
            second: "@account=guest :guest!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :hello again",
            same_person: false,
            account: Some("guest"),
        },
        Handoff {
            name: "logged out twice",
            caps: "account-tag message-tags",
            first: ":guest!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :hello",
            second: ":guest!~u@8ajhv7kxgp7ag.irc NOTICE papo-admin :hello again",
            same_person: true,
            account: None,
        },
        Handoff {
            name: "no account-tag, same host",
            caps: "message-tags",
            first: ":guest!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :hello",
            second: "@+typing=active :guest!~u@8ajhv7kxgp7ag.irc TAGMSG papo-admin",
            same_person: true,
            account: Some(""),
        },
        Handoff {
            name: "no account-tag, another host",
            caps: "message-tags",
            first: ":guest!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :hello",
            second: ":guest!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :hello again",
            same_person: false,
            account: Some(""),
        },
    ] {
        let mut session = registered(case.caps);
        let nick = case.first.split_once(':').unwrap().1;
        let nick = &nick[..nick.find('!').unwrap()];
        server(&mut session, case.first);
        let before = token(&session, nick);
        server(&mut session, case.second);
        let expected = match case.account {
            Some("") => Account::Unknown,
            Some(name) => logged_in(&session, name),
            None => Account::LoggedOut,
        };
        assert_eq!(
            token(&session, nick) == before,
            case.same_person,
            "{}",
            case.name
        );
        assert_eq!(account(&session, nick), Some(expected), "{}", case.name);
    }
}

#[test]
fn a_channel_member_without_the_account_tag_logs_out_in_place() {
    let mut session = registered("account-tag extended-join message-tags");
    for line in [
        "@account=papo-admin :papo-admin!~u@localhost JOIN #ops papo-admin :papo",
        "@account=alice :alice!~u@8ajhv7kxgp7ag.irc JOIN #ops alice :Alice Admin",
        ":ergo.test 353 papo-admin = #ops :papo-admin alice carol",
        ":ergo.test 366 papo-admin #ops :End of NAMES list",
    ] {
        server(&mut session, line);
    }
    drain(&mut session);
    let alice = token(&session, "alice");
    assert_eq!(
        account(&session, "alice"),
        Some(logged_in(&session, "alice"))
    );
    assert_eq!(account(&session, "carol"), Some(Account::Unknown));
    server(
        &mut session,
        ":carol!~u@3fx9z1q8yk2pa.irc PRIVMSG #ops :morning",
    );
    let spoke = drain(&mut session);
    assert_eq!(account(&session, "carol"), Some(Account::LoggedOut));
    assert!(
        !spoke
            .iter()
            .any(|event| matches!(event, Event::AccountChanged { .. })),
        "learning that carol is logged out is not a change: {spoke:?}"
    );

    server(
        &mut session,
        ":alice!~u@8ajhv7kxgp7ag.irc PRIVMSG #ops :brb",
    );
    let events = drain(&mut session);

    assert_eq!(account(&session, "alice"), Some(Account::LoggedOut));
    assert_eq!(token(&session, "alice"), alice);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::AccountChanged { nick, account: Account::LoggedOut, .. } if nick == "alice"
    )));
}

#[test]
fn our_own_untagged_line_keeps_the_account_from_sasl() {
    let mut session = registered("account-tag message-tags");
    server(
        &mut session,
        ":ergo.test 900 papo-admin papo-admin!~u@localhost papo-admin :You are now logged in as papo-admin",
    );
    server(&mut session, ":papo-admin!~u@localhost JOIN #ops");
    drain(&mut session);

    assert_eq!(
        session.state().account(),
        &logged_in(&session, "papo-admin")
    );
}

#[test]
fn a_who_proven_persons_first_line_keeps_the_proof_only_with_the_proven_account() {
    for (caps, line, kept) in [
        (
            "account-tag message-tags",
            "@account=bob :bob!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :thanks for the invite",
            true,
        ),
        (
            "account-tag message-tags",
            "@account=mallory :bob!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :thanks for the invite",
            false,
        ),
        (
            "account-tag message-tags",
            ":bob!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :thanks for the invite",
            false,
        ),
        (
            "message-tags",
            ":bob!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :thanks for the invite",
            false,
        ),
    ] {
        let mut session = registered(caps);
        let names = session.state().casemapping();
        let proof = session
            .__prove_who_user(
                names.nick("bob"),
                logged_in(&session, "bob"),
                &[papo_proto::Line::new("354").with_params(["papo-admin", "bob", "bob"])],
            )
            .unwrap();
        server(&mut session, line);
        assert_eq!(
            session
                .state()
                .__resolve_proven(&proof)
                .is_ok_and(|nick| nick == "bob"),
            kept,
            "{line} with {caps}"
        );
        assert_eq!(
            token(&session, "bob") == proof.__recipient().unwrap().__token(),
            kept,
            "{line} with {caps}: the sender takes over the proven identity"
        );
    }
}

#[test]
fn a_logged_out_who_proof_never_passes_to_the_next_logged_out_sender() {
    let mut session = registered("account-tag message-tags");
    let names = session.state().casemapping();
    let proof = session
        .__prove_who_user(
            names.nick("guest"),
            Account::LoggedOut,
            &[papo_proto::Line::new("354").with_params(["papo-admin", "guest", "0"])],
        )
        .unwrap();
    server(
        &mut session,
        ":guest!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :who is this?",
    );
    assert!(session.state().__resolve_proven(&proof).is_err());
}

#[test]
fn a_monitored_sender_keeps_their_identity_across_a_host_change() {
    let mut session = registered("account-tag message-tags");
    session.monitor_add(["bob"]);
    server(
        &mut session,
        ":ergo.test 730 papo-admin :bob!~u@8ajhv7kxgp7ag.irc",
    );
    drain(&mut session);
    let before = token(&session, "bob");

    server(
        &mut session,
        ":bob!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :moved to the new bastion",
    );

    assert_eq!(token(&session, "bob"), before);
}
