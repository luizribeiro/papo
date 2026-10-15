//! Transcript tests for SASL authentication.

mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_core::{Config, Entropy, Event, Sasl, Session};
use support::Transcript;

struct FixedEntropy;

impl Entropy for FixedEntropy {
    fn fill(&mut self, buf: &mut [u8]) {
        buf.fill(0x5a);
    }
}

#[test]
fn configures_mechanisms_policy_and_entropy() {
    let config = Config::default()
        .sasl(Sasl::any("alice", "correct horse battery staple"))
        .require_sasl(true)
        .device("phone");
    assert!(config.sasl.is_some());
    assert!(config.require_sasl);
    assert_eq!(config.device.as_deref(), Some("phone"));

    let session = Session::with_entropy(config, Box::new(FixedEntropy));
    assert!(format!("{session:?}").contains("has_entropy: true"));
}

#[test]
fn debug_output_redacts_server_and_sasl_passwords() {
    let config = Config::default()
        .password("server-password-7391")
        .sasl(Sasl::any("alice", "correct horse battery staple"));

    for debug in [format!("{config:?}"), format!("{:?}", Session::new(config))] {
        assert!(!debug.contains("server-password-7391"));
        assert!(!debug.contains("correct horse battery staple"));
        assert!(debug.contains("[REDACTED]"));
    }
}

#[test]
fn external_accepts_an_authorization_identity() {
    assert_ne!(Sasl::external(), Sasl::external().authzid("alice"));
}

#[test]
fn starts_sasl_after_ack_and_holds_cap_end() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .no_client()
        .server(":ergo.test 903 alice :SASL authentication successful")
        .client("CAP END");
}

#[test]
fn authenticates_with_plain_and_records_the_account() {
    Transcript::new(
        Config::default()
            .sasl(Sasl::plain("alice", "correct horse battery staple"))
            .device("phone"),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlQHBob25lAGNvcnJlY3QgaG9yc2UgYmF0dGVyeSBzdGFwbGU=")
    .server(":ergo.test 900 alice alice!~u@host alice :You are now logged in as alice")
    .inspect(|session| assert_eq!(session.account(), Some("alice")))
    .no_client()
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END");
}

#[test]
fn accepts_prefixed_plain_challenge_and_tracks_logout() {
    Transcript::new(Config::default().sasl(Sasl::plain("emersion", "hunter2")))
        .connect_default()
        .server(":solanum.example CAP * LS :sasl")
        .client("CAP REQ :sasl")
        .server(":solanum.example CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":solanum.example AUTHENTICATE +")
        .client("AUTHENTICATE AGVtZXJzaW9uAGh1bnRlcjI=")
        .server(":solanum.example 900 emersion emersion!user@host emersion :logged in")
        .inspect(|session| assert_eq!(session.account(), Some("emersion")))
        .server(":solanum.example 903 emersion :SASL authentication successful")
        .client("CAP END")
        .server(":solanum.example 901 emersion emersion!user@host :logged out")
        .inspect(|session| assert_eq!(session.account(), None));
}

#[test]
fn chunks_plain_responses_at_literal_400_bytes() {
    let mut password = "correct horse battery staple ".repeat(11);
    password.truncate(290);
    let encoded = STANDARD.encode(format!("\0emersion\0{password}"));
    assert_eq!(encoded.len(), 400);

    Transcript::new(Config::default().sasl(Sasl::plain("emersion", password)))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client(&format!("AUTHENTICATE {encoded}"))
        .client("AUTHENTICATE +")
        .server(":ergo.test 903 emersion :SASL authentication successful")
        .client("CAP END");

    let password = "battery staple ".repeat(21);
    let encoded = STANDARD.encode(format!("\0emersion\0{password}"));
    assert!(encoded.len() > 400);
    let (first, last) = encoded.split_at(400);
    assert_eq!(first.len(), 400);
    assert!(last.len() < 400);
    Transcript::new(Config::default().sasl(Sasl::plain("emersion", password)))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client(&format!("AUTHENTICATE {first}"))
        .client(&format!("AUTHENTICATE {last}"))
        .server(":ergo.test 903 emersion :SASL authentication successful")
        .client("CAP END");
}

#[test]
fn falls_back_and_honors_the_server_mechanism_list() {
    Transcript::with_entropy(
        Config::default().sasl(Sasl::any("alice", "hunter2")),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server(":ergo.test 908 alice PLAIN :are available SASL mechanisms")
    .no_client()
    .server(":ergo.test 904 alice :SASL authentication failed")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END");
}

#[test]
fn reports_every_failed_mechanism_and_continues_when_optional() {
    Transcript::new(Config::default().sasl(Sasl::any("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 905 alice :SASL message too long")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason }
                if reason.contains("SCRAM-SHA-256 skipped because no entropy")
                    && reason.contains("PLAIN"))
        })
        .client("CAP END");
}

#[test]
fn already_authenticated_counts_as_success() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 907 alice :You have already authenticated using SASL")
        .client("CAP END");
}

#[test]
fn required_sasl_closes_when_unavailable_naked_or_failed() {
    for transcript in [
        Transcript::new(
            Config::default()
                .sasl(Sasl::plain("alice", "hunter2"))
                .require_sasl(true),
        )
        .connect_default()
        .server("CAP * LS :")
        .event(|event| matches!(event, Event::Close { reason } if reason.contains("did not advertise"))),
        Transcript::new(
            Config::default()
                .sasl(Sasl::plain("alice", "hunter2"))
                .require_sasl(true),
        )
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * NAK :sasl")
        .event(|event| matches!(event, Event::Close { reason } if reason.contains("rejected"))),
        Transcript::new(
            Config::default()
                .sasl(Sasl::plain("alice", "hunter2"))
                .require_sasl(true),
        )
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 904 alice :SASL authentication failed")
        .event(|event| matches!(event, Event::Close { reason } if reason.contains("PLAIN"))),
    ] {
        transcript.no_client();
    }
}

#[test]
fn terminal_sasl_numerics_do_not_try_another_mechanism() {
    for numeric in ["902", "906"] {
        Transcript::new(Config::default().sasl(Sasl::any("alice", "hunter2")))
            .connect_default()
            .server("CAP * LS :sasl=PLAIN")
            .client("CAP REQ :sasl")
            .server("CAP * ACK :sasl")
            .client("AUTHENTICATE PLAIN")
            .server(&format!(
                ":ergo.test {numeric} alice :SASL authentication cannot continue"
            ))
            .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
            .client("CAP END");
    }
}

#[test]
fn account_required_closes_and_verification_note_surfaces() {
    Transcript::new(
        Config::default()
            .sasl(Sasl::plain("alice", "hunter2"))
            .require_sasl(true),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN standard-replies")
    .client("CAP REQ :standard-replies sasl")
    .server("CAP * ACK :standard-replies sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":ergo.test NOTE AUTHENTICATE VERIFICATION_REQUIRED alice :Verify your account")
    .event(|event| matches!(event, Event::Message(message) if message.as_ref().command_is("NOTE")))
    .server(":ergo.test FAIL * ACCOUNT_REQUIRED :Authentication required")
    .event(|event| matches!(event, Event::Close { reason } if reason.contains("Authentication required")))
    .no_client();
}

#[test]
fn aborts_at_the_literal_30_second_deadline_then_falls_back() {
    Transcript::with_entropy(
        Config::default().sasl(Sasl::any("alice", "hunter2")),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .deadline(Some(std::time::Duration::from_secs(30)))
    .advance(std::time::Duration::from_secs(29))
    .no_client()
    .deadline(Some(std::time::Duration::from_secs(1)))
    .advance(std::time::Duration::from_secs(1))
    .client("AUTHENTICATE *")
    .no_event()
    .server(":ergo.test 906 alice :SASL authentication aborted")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END");
}

#[test]
fn a_sasl_deadline_is_cleared_on_disconnect() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .deadline(Some(std::time::Duration::from_secs(30)))
        .disconnect()
        .deadline(None);
}

#[test]
fn reconnect_resets_every_sasl_exchange_state() {
    Transcript::with_entropy(
        Config::default().sasl(Sasl::any("alice", "hunter2")),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server(":ergo.test 904 alice :SASL authentication failed")
    .client("AUTHENTICATE PLAIN")
    .disconnect()
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsbj1hbGljZSxyPVdscGFXbHBhV2xwYVdscGFXbHBhV2xwYQ==")
    .disconnect()
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256");

    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 900 alice alice!~u@host alice :You are now logged in as alice")
        .server(":ergo.test 903 alice :SASL authentication successful")
        .client("CAP END")
        .inspect(|session| assert_eq!(session.account(), Some("alice")))
        .disconnect()
        .connect_default()
        .inspect(|session| assert_eq!(session.account(), None))
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN");
}

#[test]
fn authenticates_with_external_empty_and_explicit_authzids() {
    for (sasl, expected) in [
        (Sasl::external(), "AUTHENTICATE +"),
        (
            Sasl::external().authzid("alice@example.org"),
            "AUTHENTICATE YWxpY2VAZXhhbXBsZS5vcmc=",
        ),
    ] {
        Transcript::new(Config::default().sasl(sasl))
            .connect_default()
            .server("CAP * LS :sasl=EXTERNAL,PLAIN")
            .client("CAP REQ :sasl")
            .server("CAP * ACK :sasl")
            .client("AUTHENTICATE EXTERNAL")
            .server("AUTHENTICATE +")
            .client(expected)
            .server(":ergo.test 903 alice :SASL authentication successful")
            .client("CAP END");
    }
}

fn scram_start(require_sasl: bool) -> Transcript {
    Transcript::with_entropy(
        Config::default()
            .sasl(Sasl::scram_sha256("user", "pencil"))
            .require_sasl(require_sasl),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
}

fn scram_after_client_first(require_sasl: bool) -> Transcript {
    scram_start(require_sasl)
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE biwsbj11c2VyLHI9V2xwYVdscGFXbHBhV2xwYVdscGFXbHBh")
}

fn scram_after_client_final(require_sasl: bool) -> Transcript {
    scram_after_client_first(require_sasl)
        .server(concat!(
            "AUTHENTICATE ",
            "cj1XbHBhV2xwYVdscGFXbHBhV2xwYVdscGElaHZZRHBXVWEyUmFUQ0FmdXhGSWxqKWhObEYkazAs",
            "cz1XMjJaYUowU05ZN3NvRXNVRWpiNmdRPT0saT00MDk2"
        ))
        .client(concat!(
            "AUTHENTICATE ",
            "Yz1iaXdzLHI9V2xwYVdscGFXbHBhV2xwYVdscGFXbHBhJWh2WURwV1VhMlJhVENBZnV4Rklsailo",
            "TmxGJGswLHA9M2dCTVFERWRxdS9jM2FoSjkzMUwrSWl5d0N6QWpoMm92cWFaZExhYjkrWT0="
        ))
}

fn scram_until_server_final(server_final: &str) -> Transcript {
    scram_after_client_final(false).server(server_final)
}

#[test]
fn rejects_scram_success_without_a_verified_server_proof() {
    for transcript in [
        scram_start(true),
        scram_after_client_first(true),
        scram_after_client_final(true),
    ] {
        transcript
            .server(":ergo.test 903 user :SASL authentication successful")
            .client("AUTHENTICATE *")
            .event(|event| {
                matches!(event, Event::Close { reason }
                    if reason.contains("without a verified server proof"))
            })
            .no_client();
    }
}

#[test]
fn authenticates_with_scram_sha256_using_fixed_entropy() {
    scram_until_server_final(
        "AUTHENTICATE dj1hV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==",
    )
    .client("AUTHENTICATE +")
    .server(":ergo.test 903 user :SASL authentication successful")
    .client("CAP END");
}

#[test]
fn rejects_a_tampered_scram_server_signature() {
    scram_until_server_final(
        "AUTHENTICATE dj1iV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==",
    )
    .client("AUTHENTICATE *")
    .server(":ergo.test 906 user :SASL authentication aborted")
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason }
            if reason.contains("ServerSignatureMismatch"))
    })
    .client("CAP END");
}

#[test]
fn replays_recorded_ergo_plain_login() {
    Transcript::new(
        Config::default()
            .wanted_caps(Vec::<String>::new())
            .sasl(Sasl::plain("paposasl", "v3ry-Long_Unique-Password-7391")),
    )
    .connect_default()
    .load("tests/data/ergo-sasl-plain.irc")
    .inspect(|session| {
        assert_eq!(session.account(), Some("paposasl"));
        assert_eq!(session.nick(), "paposasl");
    });
}

#[test]
fn replays_recorded_ergo_scram_login() {
    Transcript::with_entropy(
        Config::default()
            .wanted_caps(Vec::<String>::new())
            .sasl(Sasl::scram_sha256(
                "paposasl",
                "v3ry-Long_Unique-Password-7391",
            )),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .load("tests/data/ergo-sasl-scram.irc")
    .inspect(|session| {
        assert_eq!(session.account(), Some("paposasl"));
        assert_eq!(session.nick(), "paposasl");
    });
}

#[test]
fn rejects_unoffered_and_908_removed_mechanisms() {
    Transcript::new(Config::default().sasl(Sasl::external()))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
        .client("CAP END");

    Transcript::with_entropy(
        Config::default().sasl(Sasl::any("alice", "hunter2")),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server(":ergo.test 908 alice SCRAM-SHA-256 :are available SASL mechanisms")
    .server(":ergo.test 904 alice :SASL authentication failed")
    .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
    .client("CAP END");
}

#[test]
fn each_authenticate_step_renews_the_deadline() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .advance(std::time::Duration::from_secs(29))
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
        .deadline(Some(std::time::Duration::from_secs(30)))
        .server(":ergo.test 903 alice :SASL authentication successful")
        .client("CAP END");
}

#[test]
fn aborts_a_challenge_without_a_payload() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE")
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 alice :SASL authentication aborted")
        .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
        .client("CAP END");
}

#[test]
fn aborts_a_server_challenge_over_4096_bytes() {
    let chunk = format!("AUTHENTICATE {}", "YWFh".repeat(100));
    let mut transcript = Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN");
    for _ in 0..10 {
        transcript = transcript.server(&chunk).no_client();
    }
    transcript
        .server(&chunk)
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 alice :SASL authentication aborted")
        .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
        .client("CAP END");
}
