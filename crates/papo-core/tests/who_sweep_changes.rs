//! Transcript tests for WHO rows that arrive after a live change to the same member.

mod support;

use papo_core::{Account, Away, Config, Event};
use papo_proto::Casemapping;
use support::Transcript;

const ALICE_ROW: &str = ":ergo.test 354 bob 152 #papo ~alice 127.0.0.1 alice H@ alice :Alice Smith";
const DAN_ROW: &str = ":ergo.test 354 bob 152 #papo ~dan 127.0.0.1 dan H dan :Dan Garcia";
const BOB_ROW: &str = ":ergo.test 354 bob 152 #papo ~bob 127.0.0.1 bob H 0 :Bob Jones";
const ALICE_QUIT: &str = ":alice!~alice@127.0.0.1 QUIT :Quit: leaving";

fn join_and_start_sweep(config: Config, isupport: &str, names: &str) -> Transcript {
    Transcript::new(config.nick("bob"))
        .server(isupport)
        .server(":bob!~bob@127.0.0.1 JOIN #papo")
        .server(names)
        .server(":ergo.test 366 bob #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo %tcuhnfar,152")
        .event_until(|event| matches!(event, Event::ChannelSynced { .. }))
}

fn sweep_in_flight(config: Config) -> Transcript {
    join_and_start_sweep(
        config,
        ":ergo.test 005 bob WHOX PREFIX=(ov)@+ :are supported",
        ":ergo.test 353 bob = #papo :@alice bob dan",
    )
}

fn finish_sweep(transcript: Transcript, rows: &[&str]) -> Transcript {
    rows.iter()
        .fold(transcript, |transcript, row| transcript.server(row))
        .server(":ergo.test 315 bob #papo :End of WHO list")
        .event_until(|event| matches!(event, Event::MembersRefreshed { .. }))
}

#[test]
fn learning_many_initial_away_statuses_emits_one_refresh_event() {
    let members = [
        "alice", "dan", "emily", "ferris", "grace", "helen", "iris", "joel", "kai", "lena",
        "marin", "nora", "omar", "pat", "quin", "rachel",
    ];
    let names = format!(":ergo.test 353 bob = #papo :@bob {}", members.join(" "));
    let mut sweep = join_and_start_sweep(
        Config::default(),
        ":ergo.test 005 bob WHOX PREFIX=(ov)@+ :are supported",
        &names,
    )
    .inspect(|session| {
        for nick in members {
            assert_eq!(session.state().user(nick).unwrap().away(), &Away::Unknown);
        }
    })
    .no_event()
    .server(":ergo.test 354 bob 152 #papo ~bob 127.0.0.1 bob H@ 0 :Bob Jones")
    .no_event();
    for (index, nick) in members.into_iter().enumerate() {
        let flag = if index % 2 == 0 { 'H' } else { 'G' };
        sweep = sweep
            .server(&format!(
                ":ergo.test 354 bob 152 #papo ~{nick} cloak.example {nick} {flag} 0 :{nick}"
            ))
            .no_event();
    }
    sweep
        .server(":ergo.test 315 bob #papo :End of WHO list")
        .event(
            |event| matches!(event, Event::MembersRefreshed { channel, .. } if channel == "#papo"),
        )
        .no_event()
        .inspect(|session| {
            for (index, nick) in members.into_iter().enumerate() {
                let expected = if index % 2 == 0 {
                    Away::Present
                } else {
                    Away::__away_value(None)
                };
                assert_eq!(session.state().user(nick).unwrap().away(), &expected);
            }
        });
}

fn logged_in(account: &str) -> Account {
    Account::LoggedIn(Casemapping::default().account(account))
}

fn member_nicks(session: &papo_core::Session) -> Vec<String> {
    let mut nicks: Vec<_> = session
        .state()
        .channel("#papo")
        .unwrap()
        .members()
        .map(|member| member.nick().to_string())
        .collect();
    nicks.sort();
    nicks
}

#[test]
fn rows_for_members_who_changed_during_the_sweep_are_not_applied() {
    let cases: [(&str, &[&str], &[&str]); 7] = [
        ("no change", &[], &["alice", "bob", "dan"]),
        (
            "nick",
            &[":alice!~alice@127.0.0.1 NICK ralice"],
            &["bob", "dan", "ralice"],
        ),
        (
            "case-only nick",
            &[":alice!~alice@127.0.0.1 NICK Alice"],
            &["Alice", "bob", "dan"],
        ),
        (
            "part",
            &[":alice!~alice@127.0.0.1 PART #papo :see you"],
            &["bob", "dan"],
        ),
        ("quit", &[ALICE_QUIT], &["bob", "dan"]),
        (
            "kick",
            &[":bob!~bob@127.0.0.1 KICK #papo alice :flooding"],
            &["bob", "dan"],
        ),
        (
            "new holder joins",
            &[ALICE_QUIT, ":alice!~mallory@203.0.113.9 JOIN #papo"],
            &["alice", "bob", "dan"],
        ),
    ];
    for (name, changes, expected) in cases {
        let transcript = changes
            .iter()
            .fold(sweep_in_flight(Config::default()), |transcript, line| {
                transcript.server(line)
            });
        finish_sweep(transcript, &[ALICE_ROW, DAN_ROW, BOB_ROW])
            .inspect(|session| {
                assert_eq!(member_nicks(session), expected, "{name}");
                let state = session.state();
                assert_eq!(state.user("dan").unwrap().account(), &logged_in("dan"));
                let alice = state.user("alice");
                let alice_row_applied =
                    alice.is_some_and(|alice| alice.account() == &logged_in("alice"));
                assert_eq!(alice_row_applied, name == "no change", "{name}");
                if name == "new holder joins" {
                    let alice = alice.unwrap();
                    let facts = (alice.username(), alice.host(), alice.realname());
                    assert_eq!(facts, (Some("~mallory"), Some("203.0.113.9"), None));
                }
            })
            .ignore_rest();
    }
}

#[test]
fn rows_are_skipped_once_changes_outgrow_the_member_limit() {
    let changes = [
        ":alice!~alice@127.0.0.1 NICK alice_",
        ":alice_!~alice@127.0.0.1 NICK alice__",
        ":alice__!~alice@127.0.0.1 PART #papo",
        ":carol!~carol@198.51.100.7 JOIN #papo",
    ];
    for (extra_change, dan_row_applied) in [
        (None, true),
        (Some(":carol!~carol@198.51.100.7 PART #papo"), false),
    ] {
        let transcript = changes.iter().chain(&extra_change).fold(
            sweep_in_flight(Config::default().max_members_per_channel(3)),
            |transcript, line| transcript.server(line),
        );
        finish_sweep(transcript, &[DAN_ROW])
            .inspect(|session| {
                let dan = session.state().user("dan").unwrap();
                assert_eq!(dan.account() == &logged_in("dan"), dan_row_applied);
            })
            .ignore_rest();
    }
}

#[test]
fn changed_members_follow_a_casemapping_change_during_the_sweep() {
    let transcript = join_and_start_sweep(
        Config::default(),
        ":ergo.test 005 bob WHOX CASEMAPPING=ascii :are supported",
        ":ergo.test 353 bob = #papo :@alice[m] bob",
    )
    .server(":alice[m]!~alice@127.0.0.1 NICK ralice")
    .server(":ergo.test 005 bob CASEMAPPING=rfc1459 :are supported");
    finish_sweep(
        transcript,
        &[":ergo.test 354 bob 152 #papo ~alice 127.0.0.1 alice[m] H@ alice :Alice Smith"],
    )
    .inspect(|session| assert_eq!(member_nicks(session), ["bob", "ralice"]))
    .ignore_rest();
}

#[test]
fn renaming_mid_sweep_skips_rows_for_both_nicknames() {
    let transcript = sweep_in_flight(Config::default()).server(":dan!~dan@127.0.0.1 NICK carol");
    finish_sweep(
        transcript,
        &[
            DAN_ROW,
            ":ergo.test 354 bob 152 #papo ~carol 198.51.100.7 carol H@ carol :Carol Diaz",
        ],
    )
    .inspect(|session| {
        let state = session.state();
        assert_eq!(member_nicks(session), ["alice", "bob", "carol"]);
        assert!(state.user("dan").is_none());
        let renamed = state.user("carol").unwrap();
        assert_ne!(renamed.account(), &logged_in("carol"));
        assert_eq!((renamed.username(), renamed.host()), (None, None));
    })
    .ignore_rest();
}
