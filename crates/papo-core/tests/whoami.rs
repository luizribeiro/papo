//! Transcript tests for `draft/whoami`.

mod support;

use papo_core::{Config, Event, TextKind};
use support::Transcript;

fn welcomed(realname: &str) -> Transcript {
    Transcript::new(
        Config::default()
            .nick("alice")
            .username("client")
            .realname(realname)
            .wanted_caps(["draft/whoami"]),
    )
    .connect()
    .client("CAP LS 302")
    .client("NICK alice")
    .client(&format!("USER client 0 * :{realname}"))
    .server("CAP * LS :draft/whoami")
    .client("CAP REQ :draft/whoami")
    .server("CAP * ACK :draft/whoami")
    .client("CAP END")
    .server(":ergo.test 001 Alice :Welcome to Ergo")
}

#[test]
fn whoami_updates_identity_without_a_spurious_realname_event() {
    welcomed("Alice Example")
        .server(":Alice!~alice@users/alice SETNAME :Alice Example")
        .no_event()
        .inspect(|session| {
            assert_eq!(session.nick(), "Alice");
            assert_eq!(session.state().nick(), "Alice");
            assert_eq!(session.state().username(), "~alice");
            assert_eq!(session.state().host(), Some("users/alice"));
        })
        .update(|session| {
            session
                .send_text(
                    std::time::Instant::now(),
                    TextKind::Privmsg,
                    "#papo",
                    &"a".repeat(470),
                    &[],
                )
                .unwrap();
        })
        .client(&format!("PRIVMSG #papo {}", "a".repeat(469)))
        .client("PRIVMSG #papo a");
}

#[test]
fn whoami_reports_an_actual_realname_change() {
    welcomed("Old Name")
        .server(":Alice!~alice@users/alice SETNAME :Alice Example")
        .event(|event| {
            matches!(event, Event::RealnameChanged { nick, realname }
            if nick == "Alice" && realname == "Alice Example")
        });
}
