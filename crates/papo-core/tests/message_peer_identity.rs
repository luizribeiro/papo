//! Transcript coverage for people papo knows only from their private lines, who can leave and be
//! replaced under the same nick without papo seeing it.

use std::time::Instant;

use papo_core::{AccountStatus, Config, Event, RecipientStatus, ReplyToken, Session};

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

fn account(session: &Session, nick: &str) -> Option<AccountStatus> {
    session
        .state()
        .user(nick)
        .map(|user| user.account().clone())
}

fn logged_in(session: &Session, name: &str) -> AccountStatus {
    AccountStatus::LoggedIn(session.state().casemapping().account(name))
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

    assert_eq!(account(&session, "alice"), Some(AccountStatus::LoggedOut));
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
            Some("") => AccountStatus::Unknown,
            Some(name) => logged_in(&session, name),
            None => AccountStatus::LoggedOut,
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
    assert_eq!(account(&session, "carol"), Some(AccountStatus::Unknown));
    server(
        &mut session,
        ":carol!~u@3fx9z1q8yk2pa.irc PRIVMSG #ops :morning",
    );
    let spoke = drain(&mut session);
    assert_eq!(account(&session, "carol"), Some(AccountStatus::LoggedOut));
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

    assert_eq!(account(&session, "alice"), Some(AccountStatus::LoggedOut));
    assert_eq!(token(&session, "alice"), alice);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::AccountChanged { nick, account: AccountStatus::LoggedOut, .. } if nick == "alice"
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

#[test]
fn continuous_private_messages_evict_the_oldest_message_only_user() {
    let mut session = registered("account-tag message-tags");
    let now = Instant::now();
    session.handle_line(
        now,
        b":alicia!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :signing off",
    );
    session.handle_line(now, b":alicia!~u@kq2ccwrbbbw6a.irc QUIT :Client Quit");
    session.handle_line(
        now,
        b":carol!~u@users.example PRIVMSG papo-admin :please review the release",
    );
    session.handle_line(now, ALICE_ADMIN.as_bytes());
    session.handle_line(now, b":alice!~u@8ajhv7kxgp7ag.irc NICK alicia");
    let nick = session.state().casemapping().nick("alicia");
    let guard = session
        .state()
        .__capture_recipient(nick.clone(), AccountStatus::Unknown)
        .unwrap();
    for index in 0..1_050 {
        let line =
            format!(":guest{index}!~u@users.example PRIVMSG papo-admin :is the release out?");
        session.handle_line(now, line.as_bytes());
        drain(&mut session);
        assert_eq!(
            session.state().user(nick.as_str()).is_some(),
            index < 999,
            "{index}"
        );
        assert_eq!(
            session.state().user("carol").is_some(),
            index < 998,
            "{index}"
        );
        assert!(session.state().user(format!("guest{index}")).is_some());
    }
    assert!(!matches!(
        session.state().__resolve_recipient(&guard),
        RecipientStatus::Current(_)
    ));
}

#[test]
fn monitor_sync_preserves_a_message_only_identity() {
    let mut failures = Vec::new();
    for (nick, line) in [
        (Some("bob"), None),
        (
            Some("alice"),
            Some(":ergo.test 730 papo-admin :alice!~u@8ajhv7kxgp7ag.irc"),
        ),
        (
            None,
            Some(":ergo.test 005 papo-admin NETWORK=ErgoTest :are supported"),
        ),
    ] {
        let mut session = registered("account-tag message-tags");
        let mut now = Instant::now();
        session.handle_line(now, b":ergo.test 005 papo-admin MONITOR=100 :are supported");
        now += std::time::Duration::from_millis(1);
        session.handle_line(now, ALICE_ADMIN.as_bytes());
        drain(&mut session);
        let before = token(&session, "alice");
        if let Some(nick) = nick {
            session.monitor_add([nick]);
            assert_eq!(
                session.poll_transmit().unwrap().as_ref(),
                format!("MONITOR + {nick}\r\n").as_bytes()
            );
        }
        if let Some(line) = line {
            now += std::time::Duration::from_millis(1);
            session.handle_line(now, line.as_bytes());
        }
        if token(&session, "alice") != before {
            failures.push((nick, line));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

fn observe_after_private_message(evidence: &str) -> (bool, RecipientStatus) {
    let mut session = registered("message-tags account-notify chghost userhost-in-names");
    for line in [
        ":ergo.test 005 papo-admin MONITOR=100 WHOX :are supported",
        ":papo-admin!~u@localhost JOIN #review",
        ":ergo.test 366 papo-admin #review :End of NAMES list",
        ALICE_ADMIN,
    ] {
        server(&mut session, line);
    }
    for expected in ["MODE #review\r\n", "WHO #review %tcuhnfar,152\r\n"] {
        assert_eq!(
            session.poll_transmit().unwrap().as_ref(),
            expected.as_bytes()
        );
    }
    assert!(
        drain(&mut session)
            .iter()
            .any(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("PRIVMSG")))
    );
    let before = token(&session, "alice");
    let guard = session
        .state()
        .__capture_recipient(
            session.state().casemapping().nick("alice"),
            AccountStatus::Unknown,
        )
        .unwrap();
    if matches!(
        evidence.split_whitespace().nth(1),
        Some("730" | "731" | "734")
    ) {
        session.monitor_add(["alice"]);
        assert_eq!(
            session.poll_transmit().unwrap().as_ref(),
            b"MONITOR + alice\r\n"
        );
    }
    let now = Instant::now() + std::time::Duration::from_millis(1);
    session.handle_line(now, evidence.as_bytes());
    (
        before == token(&session, "alice"),
        session.state().__resolve_recipient(&guard),
    )
}

#[test]
#[rustfmt::skip]
fn message_only_identity_uses_live_user_lines() {
    for (evidence, same_person) in [
        (":alice!~u@kq2ccwrbbbw6a.irc PRIVMSG papo-admin :hello", false),
        (":alice!~u@kq2ccwrbbbw6a.irc JOIN #review", false),
        (":alice!~u@8ajhv7kxgp7ag.irc JOIN #review", true),
        (":alice!~u@kq2ccwrbbbw6a.irc JOIN #review alice :Alice", false),
        (":alice!~u@8ajhv7kxgp7ag.irc JOIN #review mallory :Mallory", false),
        (":alice!~u@8ajhv7kxgp7ag.irc JOIN #review alice :Alice", true),
        (":alice!~u@8ajhv7kxgp7ag.irc CHGHOST ~u kq2ccwrbbbw6a.irc", false),
        (":alice!~u@8ajhv7kxgp7ag.irc ACCOUNT mallory", false),
        (":alice!~u@8ajhv7kxgp7ag.irc ACCOUNT alice", true),
        (":alice!~u@kq2ccwrbbbw6a.irc ACCOUNT alice", false),
        ("@account=alice :alice!~u@8ajhv7kxgp7ag.irc ACCOUNT mallory", false),
        ("@account=alice :alice!~u@8ajhv7kxgp7ag.irc JOIN #review mallory :Mallory", false),
        ("@account=mallory :alice!~u@8ajhv7kxgp7ag.irc ACCOUNT alice", false),
        ("@account=mallory :alice!~u@8ajhv7kxgp7ag.irc JOIN #review alice :Alice", false),
        ("@account=mallory :alice!~u@8ajhv7kxgp7ag.irc INVITE papo-admin #staff", false),
        ("@account=alice :alice!~u@kq2ccwrbbbw6a.irc INVITE papo-admin #staff", false),
        ("@account=alice :alice!~u@8ajhv7kxgp7ag.irc INVITE papo-admin #staff", true),
    ] {
        let (same_token, status) = observe_after_private_message(evidence);
        assert_eq!(same_token, same_person, "{evidence}");
        assert_eq!(matches!(status, RecipientStatus::Current(_)), same_person, "{evidence}: {status:?}");
    }
}

#[test]
fn bot_tags_follow_the_reconciled_account_command_actor() {
    for command in ["ACCOUNT mallory", "JOIN #review mallory :Mallory"] {
        let mut session = registered("message-tags account-notify account-tag extended-join");
        server(&mut session, ALICE_ADMIN);
        let before = token(&session, "alice");
        let line = format!("@bot;account=alice :alice!~u@8ajhv7kxgp7ag.irc {command}");
        server(&mut session, &line);
        assert_ne!(token(&session, "alice"), before, "{line}");
        let user = session.state().user("alice").unwrap();
        assert_eq!(user.account(), &AccountStatus::Unknown, "{line}");
        assert_eq!(user.is_bot(), Some(true), "{line}");
    }
}

#[test]
#[rustfmt::skip]
fn message_only_identity_uses_names_and_monitor_rows() {
    let mut mismatches = Vec::new();
    for (evidence, same_person) in [
        (":ergo.test 353 papo-admin = #review :@alice!~u@kq2ccwrbbbw6a.irc", false),
        (":ergo.test 353 papo-admin = #review :@alice!~u@8ajhv7kxgp7ag.irc", true),
        (":ergo.test 730 papo-admin :alice!~u@kq2ccwrbbbw6a.irc", false),
        (":ergo.test 730 papo-admin :alice!~u@8ajhv7kxgp7ag.irc", true),
        (":ergo.test 731 papo-admin :alice", false),
        (":ergo.test 734 papo-admin 100 alice :Monitor list is full.", true),
    ] {
        let (same_token, status) = observe_after_private_message(evidence);
        if same_token != same_person || matches!(status, RecipientStatus::Current(_)) != same_person {
            mismatches.push((evidence, same_token, status));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:?}");
}

#[test]
fn message_only_identity_uses_who_whois_and_userhost_rows() {
    let mut mismatches = Vec::new();
    for evidence in [
        ":ergo.test 352 papo-admin #review ~u kq2ccwrbbbw6a.irc ergo.test alice H :0 Alice",
        ":ergo.test 352 papo-admin * ~u kq2ccwrbbbw6a.irc ergo.test alice H :0 Alice",
        ":ergo.test 354 papo-admin 152 #review ~u kq2ccwrbbbw6a.irc alice H alice :Alice",
        ":ergo.test 354 papo-admin 152 #review ~u 8ajhv7kxgp7ag.irc alice H mallory :Mallory",
        ":ergo.test 354 papo-admin 152 * ~u 8ajhv7kxgp7ag.irc alice H mallory :Mallory",
        ":ergo.test 311 papo-admin alice ~u kq2ccwrbbbw6a.irc * :Alice",
        ":ergo.test 330 papo-admin alice mallory :is logged in as",
        ":ergo.test 302 papo-admin :bob=+~u@localhost alice*=-~u@kq2ccwrbbbw6a.irc",
        ":ergo.test 302 papo-admin :alice=+kq2ccwrbbbw6a.irc",
    ] {
        let result = observe_after_private_message(evidence);
        if result != (false, RecipientStatus::Gone) {
            mismatches.push((evidence, result));
        }
    }
    for evidence in [
        ":ergo.test 311 papo-admin alice ~u 8ajhv7kxgp7ag.irc * :Alice",
        ":ergo.test 330 papo-admin alice alice :is logged in as",
        ":ergo.test 302 papo-admin :alice=+~u@8ajhv7kxgp7ag.irc",
        ":ergo.test 302 papo-admin :alice=~u@kq2ccwrbbbw6a.irc",
    ] {
        let (same_token, status) = observe_after_private_message(evidence);
        assert!(
            same_token && matches!(status, RecipientStatus::Current(_)),
            "{evidence}"
        );
    }
    assert!(mismatches.is_empty(), "{mismatches:?}");
}

#[test]
fn server_replies_fill_only_the_missing_message_sender_facts() {
    for first in [
        ":alice@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :release status",
        ":alice!~u PRIVMSG papo-admin :release status",
    ] {
        let mut session = registered("message-tags account-notify");
        let now = Instant::now();
        let frame = b":ergo.test 302 papo-admin :alice=+~u@8ajhv7kxgp7ag.irc";
        session.handle_line(now, first.as_bytes());
        let before = token(&session, "alice");
        session.handle_line(now, frame);
        let alice = session.state().user("alice").unwrap();
        assert_eq!(alice.username(), Some("~u"), "{first}");
        assert_eq!(alice.host(), Some("8ajhv7kxgp7ag.irc"), "{first}");
        assert_eq!(token(&session, "alice"), before);

        let generation = session.state_generation();
        session.handle_line(now, frame);
        assert_eq!(session.state_generation(), generation);

        session.handle_line(
            now,
            b":ergo.test 302 papo-admin :alice=+~mallory@8ajhv7kxgp7ag.irc",
        );
        assert_ne!(token(&session, "alice"), before);
    }
}

#[test]
fn whois_teaches_the_account_of_a_sender_with_unknown_login_state() {
    let mut session = registered("message-tags account-notify");
    let now = Instant::now();
    session.handle_line(
        now,
        b":alice!~u@8ajhv7kxgp7ag.irc PRIVMSG papo-admin :release status",
    );
    let before = token(&session, "alice");
    assert_eq!(account(&session, "alice"), Some(AccountStatus::Unknown));

    session.handle_line(
        now,
        b":ergo.test 330 papo-admin alice alice :is logged in as",
    );
    assert_eq!(
        account(&session, "alice"),
        Some(logged_in(&session, "alice"))
    );
    assert_eq!(token(&session, "alice"), before);

    session.handle_line(now, b":alice!~u@8ajhv7kxgp7ag.irc ACCOUNT mallory");
    assert_ne!(token(&session, "alice"), before);
}
