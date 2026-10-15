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
