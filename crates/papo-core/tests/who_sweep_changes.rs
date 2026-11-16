//! Transcript tests for WHO rows that arrive after a live change to the same member.

mod support;

use papo_core::{Account, Config, Event};
use papo_proto::Casemapping;
use support::Transcript;

const ALICE_ROW: &str = ":ergo.test 354 bob 152 #papo ~alice 127.0.0.1 alice H@ alice :Alice Smith";
const DAN_ROW: &str = ":ergo.test 354 bob 152 #papo ~dan 127.0.0.1 dan H dan :Dan Garcia";
const BOB_ROW: &str = ":ergo.test 354 bob 152 #papo ~bob 127.0.0.1 bob H 0 :Bob Jones";
const ALICE_QUIT: &str = ":alice!~alice@127.0.0.1 QUIT :Quit: leaving";

fn sweep_in_flight(config: Config) -> Transcript {
    Transcript::new(config.nick("bob"))
        .server(":ergo.test 005 bob WHOX PREFIX=(ov)@+ :are supported")
        .server(":bob!~bob@127.0.0.1 JOIN #papo")
        .server(":ergo.test 353 bob = #papo :@alice bob dan")
        .server(":ergo.test 366 bob #papo :End of NAMES list")
        .client("MODE #papo")
        .client("WHO #papo %tcuhnfar,152")
        .event_until(|event| matches!(event, Event::ChannelSynced { .. }))
}

fn finish_sweep(transcript: Transcript, rows: &[&str]) -> Transcript {
    rows.iter()
        .fold(transcript, |transcript, row| transcript.server(row))
        .server(":ergo.test 315 bob #papo :End of WHO list")
        .event_until(|event| matches!(event, Event::MembersRefreshed { .. }))
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
