//! Transcript tests for SASL authentication.

mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_core::{
    AccountStatus, CloseKind, Config, Entropy, Event, Sasl, Session, cap, sts::ConnectionSecurity,
};
use support::{Transcript, assert_session_debug_redacts, queue_debug_marker};

const OAUTH_BEARER_INVALID_TOKEN: &str = concat!(
    "AUTHENTICATE ",
    "eyJzdGF0dXMiOiJpbnZhbGlkX3Rva2VuIiwic2NoZW1lcyI6ImJlYXJlciIsInNjb3BlIjoiIn0="
);

struct FixedEntropy;

impl Entropy for FixedEntropy {
    fn fill(&mut self, buf: &mut [u8]) {
        buf.fill(0x5a);
    }
}

fn secure_sasl(sasl: Sasl) -> Config {
    Config::default()
        .connection_security(ConnectionSecurity::Secure)
        .sasl(sasl)
}

fn oauth_bearer_after_initial_response() -> Transcript {
    Transcript::new(
        Config::default().sasl(
            Sasl::oauth_bearer(papo_core::Secret::new("expired-oauth-token-4821")).optional(),
        ),
    )
    .connect_default()
    .server("CAP * LS :sasl=OAUTHBEARER")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE OAUTHBEARER")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsAWF1dGg9QmVhcmVyIGV4cGlyZWQtb2F1dGgtdG9rZW4tNDgyMQEB")
}

#[test]
fn configures_mechanisms_policy_and_entropy() {
    let config = Config::default().sasl(Sasl::password(
        "alice",
        papo_core::Secret::new("correct horse battery staple"),
    ));
    assert!(config.sasl.is_some());

    let session = Session::with_entropy(config, Box::new(FixedEntropy));
    assert!(format!("{session:?}").contains("has_entropy: true"));
}

#[test]
fn debug_output_redacts_server_and_sasl_passwords() {
    let config = Config::default()
        .password(papo_core::Secret::new("server-password-7391"))
        .sasl(Sasl::password(
            "alice",
            papo_core::Secret::new("correct horse battery staple"),
        ));

    for debug in [format!("{config:?}"), format!("{:?}", Session::new(config))] {
        assert!(!debug.contains("server-password-7391"));
        assert!(!debug.contains("correct horse battery staple"));
        assert!(debug.contains("[REDACTED]"));
    }
}

#[test]
fn debug_output_redacts_queued_pass_and_plain_response() {
    let mut session = Session::new(
        secure_sasl(Sasl::plain(
            "alice",
            papo_core::Secret::new("plain-password-4821"),
        ))
        .password(papo_core::Secret::new("server-password-7391")),
    );
    session.handle_connected(std::time::Instant::now());
    queue_debug_marker(&mut session);
    assert_session_debug_redacts(&session, "server-password-7391");
    session.handle_line(std::time::Instant::now(), b"CAP * LS :sasl=PLAIN");
    session.handle_line(std::time::Instant::now(), b"CAP * ACK :sasl");
    session.handle_line(std::time::Instant::now(), b"AUTHENTICATE +");
    assert_session_debug_redacts(&session, "plain-password-4821");
}

#[test]
fn oauth_bearer_sends_an_rfc_7628_response_and_redacts_its_token() {
    let config = Config::default().sasl(Sasl::oauth_bearer(papo_core::Secret::new(
        "oauth-token-7391",
    )));
    for debug in [
        format!("{config:?}"),
        format!("{:?}", Session::new(config.clone())),
    ] {
        assert!(!debug.contains("oauth-token-7391"));
        assert!(debug.contains("[REDACTED]"));
    }

    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :sasl=OAUTHBEARER")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE OAUTHBEARER")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE biwsAWF1dGg9QmVhcmVyIG9hdXRoLXRva2VuLTczOTEBAQ==")
        .server(":ergo.test 903 alice :SASL authentication successful")
        .client("CAP END");
}

#[test]
fn oauth_bearer_publishes_the_pending_account_only_after_success() {
    Transcript::new(
        Config::default().sasl(Sasl::oauth_bearer(papo_core::Secret::new(
            "oauth-token-7391",
        ))),
    )
    .connect_default()
    .server("CAP * LS :sasl=OAUTHBEARER")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE OAUTHBEARER")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsAWF1dGg9QmVhcmVyIG9hdXRoLXRva2VuLTczOTEBAQ==")
    .server(":ergo.test 900 papo papo!~papo@localhost alice :You are now logged in as alice")
    .no_event()
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert!(matches!(
            session.state().account(),
            AccountStatus::Unknown | AccountStatus::LoggedOut
        ));
    })
    .server(":ergo.test 903 papo :SASL authentication successful")
    .client("CAP END")
    .no_event()
    .inspect(|session| {
        assert_eq!(session.account(), Some("alice"));
        assert!(
            matches!(session.state().account(), AccountStatus::LoggedIn(name) if name == "alice")
        );
    });
}

#[test]
fn oauth_bearer_failure_discards_the_pending_account() {
    Transcript::new(
        Config::default().sasl(
            Sasl::oauth_bearer(papo_core::Secret::new("expired-oauth-token-4821")).optional(),
        ),
    )
    .connect_default()
    .server("CAP * LS :sasl=OAUTHBEARER")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE OAUTHBEARER")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsAWF1dGg9QmVhcmVyIGV4cGlyZWQtb2F1dGgtdG9rZW4tNDgyMQEB")
    .server(":ergo.test 900 papo papo!~papo@localhost alice :You are now logged in as alice")
    .no_event()
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert!(matches!(
            session.state().account(),
            AccountStatus::Unknown | AccountStatus::LoggedOut
        ));
    })
    .server(OAUTH_BEARER_INVALID_TOKEN)
    .client("AUTHENTICATE AQ==")
    .no_event()
    .server(":ergo.test 904 papo :SASL authentication failed")
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason, .. }
            if reason.contains("SASL authentication failed")
                && !reason.contains("invalid_token"))
    })
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert!(matches!(
            session.state().account(),
            AccountStatus::Unknown | AccountStatus::LoggedOut
        ));
    });
}

#[test]
fn oauth_bearer_answers_only_one_error_challenge() {
    oauth_bearer_after_initial_response()
        .server(OAUTH_BEARER_INVALID_TOKEN)
        .client("AUTHENTICATE AQ==")
        .server(OAUTH_BEARER_INVALID_TOKEN)
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 papo :SASL authentication aborted")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("second OAUTHBEARER error challenge"))
        })
        .client("CAP END");
}

#[test]
fn oauth_bearer_rejects_a_malformed_error_challenge() {
    oauth_bearer_after_initial_response()
        .server("AUTHENTICATE not-base64!")
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 papo :SASL authentication aborted")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("invalid SASL challenge"))
        })
        .client("CAP END");
}

#[test]
fn oauth_bearer_error_challenge_is_bounded_at_4096_bytes() {
    let full_chunk = format!("AUTHENTICATE {}", "YWFh".repeat(100));

    let mut transcript = oauth_bearer_after_initial_response();
    for _ in 0..10 {
        transcript = transcript.server(&full_chunk).no_client();
    }
    transcript
        .server(&format!("AUTHENTICATE {}", "YWFh".repeat(24)))
        .client("AUTHENTICATE AQ==")
        .server(":ergo.test 904 papo :SASL authentication failed")
        .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
        .client("CAP END");

    let mut transcript = oauth_bearer_after_initial_response();
    for _ in 0..10 {
        transcript = transcript.server(&full_chunk).no_client();
    }
    transcript
        .server(&format!("AUTHENTICATE {}", "YWFh".repeat(25)))
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 papo :SASL authentication aborted")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("invalid SASL challenge"))
        })
        .client("CAP END");
}

#[test]
fn oauth_bearer_error_outcome_has_a_bounded_deadline() {
    oauth_bearer_after_initial_response()
        .server(OAUTH_BEARER_INVALID_TOKEN)
        .client("AUTHENTICATE AQ==")
        .deadline(Some(std::time::Duration::from_secs(30)))
        .advance(std::time::Duration::from_secs(30))
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 papo :SASL authentication aborted")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("timed out after 30 seconds"))
        })
        .client("CAP END");
}

#[test]
fn oauth_bearer_never_accepts_success_after_an_error_challenge() {
    oauth_bearer_after_initial_response()
        .server(":ergo.test 900 papo papo!~papo@localhost alice :You are now logged in as alice")
        .server(OAUTH_BEARER_INVALID_TOKEN)
        .client("AUTHENTICATE AQ==")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("succeeded after an error challenge"))
        })
        .client("CAP END")
        .inspect(|session| {
            assert_eq!(session.account(), None);
            assert!(matches!(
                session.state().account(),
                AccountStatus::Unknown | AccountStatus::LoggedOut
            ));
        });
}

#[test]
fn debug_output_redacts_a_queued_oauth_bearer_response() {
    Transcript::new(secure_sasl(Sasl::oauth_bearer(papo_core::Secret::new(
        "oauth-token-7391",
    ))))
    .connect_default()
    .server("CAP * LS :sasl=OAUTHBEARER")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE OAUTHBEARER")
    .server("AUTHENTICATE +")
    .update(queue_debug_marker)
    .inspect(|session| assert_session_debug_redacts(session, "oauth-token-7391"))
    .client("AUTHENTICATE biwsAWF1dGg9QmVhcmVyIG9hdXRoLXRva2VuLTczOTEBAQ==")
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END")
    .server(":ergo.test 001 alice :Welcome to the Ergo test network")
    .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("001")))
    .client("PRIVMSG #papo :ordinary debug marker");
}

#[test]
fn reauthentication_without_credentials_is_a_no_op() {
    Transcript::new(Config::default())
        .reauthenticate()
        .no_event()
        .no_client();
}

#[test]
fn reauthentication_does_not_interrupt_an_active_exchange() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .reauthenticate()
    .no_event()
    .no_client()
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":ergo.test 900 papo papo!u@host alice :logged in")
    .server(":ergo.test 903 papo :SASL authentication successful")
    .client("CAP END")
    .inspect(|session| assert_eq!(session.account(), Some("alice")));
}

#[test]
fn sasl_capability_return_does_not_restart_successful_authentication() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":ergo.test 900 papo papo!u@host alice :logged in")
    .server(":ergo.test 903 papo :SASL authentication successful")
    .client("CAP END")
    .server("CAP papo DEL :sasl")
    .event(|event| {
        matches!(event, Event::CapsChanged { removed, .. }
                if removed.contains_key(&cap::SASL))
    })
    .server("CAP papo NEW :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP papo ACK :sasl")
    .event(|event| {
        matches!(event, Event::CapsChanged { added, .. }
                if added.contains_key(&cap::SASL))
    })
    .inspect(|session| assert_eq!(session.account(), Some("alice")))
    .no_client();
}

#[test]
fn bouncer_sasl_return_after_an_unnamed_success_sends_no_credentials() {
    for success in [
        ":bouncer.papo.test 903 papo :SASL authentication successful",
        ":bouncer.papo.test 907 papo :You have already authenticated using SASL",
    ] {
        Transcript::new(
            Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
        )
        .connect_default()
        .server(":bouncer.papo.test CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server(":bouncer.papo.test CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
        .server(success)
        .client("CAP END")
        .server(":bouncer.papo.test CAP * DEL sasl")
        .event(|event| {
            matches!(event, Event::CapsChanged { removed, .. }
                    if removed.contains_key(&cap::SASL))
        })
        .welcome(":bouncer.papo.test 001 papo :Welcome to soju, papo")
        .server(":bouncer.papo.test 422 papo :No MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
        .server(":bouncer.papo.test CAP papo NEW sasl=PLAIN,ANONYMOUS")
        .client("CAP REQ :sasl")
        .server(":bouncer.papo.test CAP papo ACK sasl")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, .. }
                    if added.contains_key(&cap::SASL))
        })
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, .. }
                    if added.contains(&papo_core::Feature::Sasl))
        })
        .no_client()
        .no_event()
        .reauthenticate()
        .client("AUTHENTICATE PLAIN");
    }
}

#[test]
fn bouncer_sasl_return_after_a_failed_login_sends_no_credentials() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2")).optional()),
    )
    .connect_default()
    .server(":bouncer.papo.test CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server(":bouncer.papo.test CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":bouncer.papo.test 904 papo :Invalid username or password")
    .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
    .client("CAP END")
    .server(":bouncer.papo.test CAP * DEL sasl")
    .event(|event| {
        matches!(event, Event::CapsChanged { removed, .. }
                if removed.contains_key(&cap::SASL))
    })
    .welcome(":bouncer.papo.test 001 papo :Welcome to soju, papo")
    .server(":bouncer.papo.test 422 papo :No MOTD")
    .event(|event| matches!(event, Event::Connected(_)))
    .server(":bouncer.papo.test CAP papo NEW sasl=PLAIN,ANONYMOUS")
    .client("CAP REQ :sasl")
    .server(":bouncer.papo.test CAP papo ACK sasl")
    .event(|event| {
        matches!(event, Event::CapsChanged { added, .. }
                if added.contains_key(&cap::SASL))
    })
    .event(|event| {
        matches!(event, Event::FeaturesChanged { added, .. }
                if added.contains(&papo_core::Feature::Sasl))
    })
    .no_client()
    .no_event();
}

fn scram_then_plain_started() -> Transcript {
    Transcript::with_entropy(
        secure_sasl(Sasl::password("alice", papo_core::Secret::new("hunter2"))),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server(":ergo.test CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server(":ergo.test CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
}

#[test]
fn plain_never_follows_an_answered_scram_challenge() {
    Transcript::with_entropy(
        secure_sasl(Sasl::password("user", papo_core::Secret::new("pencil")).optional()),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server(":ergo.test CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server(":ergo.test CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsbj11c2VyLHI9V2xwYVdscGFXbHBhV2xwYVdscGFXbHBh")
    .server(":ergo.test 904 papo :SASL authentication failed")
    .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
    .client("CAP END")
    .server(":ergo.test CAP * DEL :sasl")
    .event(|event| {
        matches!(event, Event::CapsChanged { removed, .. } if removed.contains_key(&cap::SASL))
    })
    .server(":ergo.test CAP * NEW :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server(":ergo.test CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .event(|event| {
        matches!(event, Event::CapsChanged { added, .. } if added.contains_key(&cap::SASL))
    })
    .server(":ergo.test 904 papo :SASL authentication failed")
    .no_client()
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason, .. }
            if reason.contains("PLAIN fallback"))
    })
    .welcome(":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo")
    .server(":ergo.test 422 papo :MOTD File is missing")
    .event(|event| matches!(event, Event::Connected(_)))
    .reauthenticate()
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server(":ergo.test 904 papo :SASL authentication failed")
    .no_client()
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason, .. }
            if reason.contains("PLAIN fallback"))
    })
    .disconnect()
    .connect_default()
    .server(":ergo.test CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server(":ergo.test CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server(":ergo.test 904 papo :SASL authentication failed")
    .client("AUTHENTICATE PLAIN");
}

fn ack_sasl_again(transcript: Transcript) -> Transcript {
    transcript
        .server(":ergo.test CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
        .client("CAP REQ :sasl")
        .server(":ergo.test CAP * ACK :sasl")
        .no_client()
        .no_event()
}

#[test]
fn a_welcome_during_the_exchange_ends_mechanism_fallback() {
    scram_then_plain_started()
        .welcome(":ergo.test 001 alice :Welcome to the ErgoTest IRC Network alice")
        .server(":ergo.test 904 alice :SASL authentication failed")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
                if reason.contains("every configured SASL mechanism failed"))
        })
        .no_client();
}

#[test]
fn a_repeated_sasl_ack_does_not_start_the_next_mechanism_early() {
    ack_sasl_again(scram_then_plain_started())
        .server(":ergo.test 904 alice :SASL authentication failed")
        .client("AUTHENTICATE PLAIN");

    ack_sasl_again(
        scram_then_plain_started()
            .advance(std::time::Duration::from_secs(30))
            .client("AUTHENTICATE *"),
    )
    .server(":ergo.test 906 alice :SASL authentication aborted")
    .client("AUTHENTICATE PLAIN");
}

fn sasl_ack_beside_a_batched_welcome(batch_type: &str) -> Transcript {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))))
        .connect_default()
        .server(":bouncer.papo.test CAP * LS :batch sasl=PLAIN")
        .client("CAP REQ :batch sasl")
        .server(&format!(":bouncer.papo.test BATCH +welcome {batch_type}"))
        .server("@batch=welcome :bouncer.papo.test 001 papo :Welcome to soju, papo")
        .server(":bouncer.papo.test CAP * ACK :batch sasl")
        .no_client()
        .server(":bouncer.papo.test BATCH -welcome")
        .no_client()
}

#[test]
fn sasl_ack_while_a_batch_holds_the_welcome_sends_no_credentials() {
    sasl_ack_beside_a_batched_welcome("metadata")
        .event(|event| matches!(event, Event::Line(line) if line.as_ref().command_is("001")))
        .no_event();
}

#[test]
fn sasl_ack_beside_a_welcome_a_batch_drops_sends_no_credentials() {
    sasl_ack_beside_a_batched_welcome("draft/isupport").no_event();
}

#[test]
fn starts_sasl_after_ack_and_holds_cap_end() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
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
fn inactive_authenticate_messages_are_surfaced_unchanged() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("AUTHENTICATE +")
    .event(|event| {
        matches!(event, Event::Line(message)
                if message.as_ref().command_is("AUTHENTICATE"))
    })
    .no_client()
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":ergo.test 900 alice alice!~u@host alice :You are now logged in as alice")
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END")
    .server("AUTHENTICATE +")
    .event(|event| {
        matches!(event, Event::Line(message)
                if message.as_ref().command_is("AUTHENTICATE"))
    })
    .inspect(|session| assert_eq!(session.account(), Some("alice")))
    .no_client();
}

#[test]
fn authenticates_with_plain_and_records_the_account() {
    Transcript::new(Config::default().sasl(Sasl::plain(
        "alice@phone",
        papo_core::Secret::new("correct horse battery staple"),
    )))
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE AGFsaWNlQHBob25lAGNvcnJlY3QgaG9yc2UgYmF0dGVyeSBzdGFwbGU=")
    .server(":ergo.test 900 alice alice!~u@host alice :You are now logged in as alice")
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert!(matches!(
            session.state().account(),
            AccountStatus::Unknown | AccountStatus::LoggedOut
        ));
    })
    .no_client()
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(session.account(), Some("alice"));
        assert!(
            matches!(session.state().account(), AccountStatus::LoggedIn(name) if name == "alice")
        );
    });
}

#[test]
fn accepts_prefixed_plain_challenge_and_tracks_logout() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("emersion", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server(":solanum.example CAP * LS :sasl")
    .client("CAP REQ :sasl")
    .server(":solanum.example CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":solanum.example AUTHENTICATE +")
    .client("AUTHENTICATE AGVtZXJzaW9uAGh1bnRlcjI=")
    .server(":solanum.example 900 emersion emersion!user@host emersion :logged in")
    .server(":solanum.example 903 emersion :SASL authentication successful")
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(session.account(), Some("emersion"));
        assert!(
            matches!(session.state().account(), AccountStatus::LoggedIn(name) if name == "emersion")
        );
    })
    .server(":solanum.example 901 emersion emersion!user@host :logged out")
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert_eq!(session.state().account(), &AccountStatus::LoggedOut);
    });
}

#[test]
fn chunks_plain_responses_at_literal_400_bytes() {
    let mut password = "correct horse battery staple ".repeat(11);
    password.truncate(290);
    let encoded = STANDARD.encode(format!("\0emersion\0{password}"));
    assert_eq!(encoded.len(), 400);

    Transcript::new(
        Config::default().sasl(Sasl::plain("emersion", papo_core::Secret::new(password))),
    )
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
    Transcript::new(
        Config::default().sasl(Sasl::plain("emersion", papo_core::Secret::new(password))),
    )
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
        secure_sasl(Sasl::password("alice", papo_core::Secret::new("hunter2"))),
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

fn any_scram_start(security: ConnectionSecurity) -> Transcript {
    Transcript::with_entropy(
        Config::default()
            .connection_security(security)
            .sasl(Sasl::password("alice", papo_core::Secret::new("hunter2"))),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
}

#[test]
fn scram_fallback_requires_secure_transport_before_a_challenge() {
    for transcript in [
        any_scram_start(ConnectionSecurity::Plaintext),
        any_scram_start(ConnectionSecurity::Plaintext)
            .server(":ergo.test 908 alice PLAIN :are available SASL mechanisms")
            .no_client(),
    ] {
        transcript
            .server(":ergo.test 904 alice :SASL authentication failed")
            .event(|event| {
                matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
                    if reason.contains("PLAIN fallback"))
            })
            .no_client();
    }

    any_scram_start(ConnectionSecurity::Secure)
        .server(":ergo.test 904 alice :SASL authentication failed")
        .client("AUTHENTICATE PLAIN");

    any_scram_start(ConnectionSecurity::Secure)
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE biwsbj1hbGljZSxyPVdscGFXbHBhV2xwYVdscGFXbHBhV2xwYQ==")
        .server(":ergo.test 904 alice :SASL authentication failed")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
                if reason.contains("PLAIN fallback"))
        })
        .no_client();
}

#[test]
fn reports_every_failed_mechanism_and_continues_when_optional() {
    Transcript::new(secure_sasl(
        Sasl::password("alice", papo_core::Secret::new("hunter2")).optional(),
    ))
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":ergo.test 905 alice :SASL message too long")
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("SCRAM-SHA-256 skipped because no entropy")
                    && reason.contains("PLAIN"))
    })
    .client("CAP END");
}

#[test]
fn already_authenticated_counts_as_success() {
    for success in [
        ":ergo.test 907 alice :You have already authenticated using SASL",
        ":bouncer.papo.test 903 papo :SASL authentication successful",
    ] {
        Transcript::new(
            Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
        )
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(success)
        .client("CAP END")
        .inspect(|session| assert_eq!(session.state().account(), &AccountStatus::Unknown));
    }
}

#[test]
fn an_unnamed_reauthentication_keeps_the_known_account() {
    registered_with_required_sasl()
        .reauthenticate()
        .client("AUTHENTICATE PLAIN")
        .server(":soju.test 907 papo :You have already authenticated using SASL")
        .no_client()
        .inspect(|session| assert_eq!(session.account(), Some("alice")));
}

fn registered_with_required_sasl() -> Transcript {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
        .server(":soju.test 900 papo papo!u@host alice :You are now logged in as alice")
        .server(":soju.test 903 papo :SASL authentication successful")
        .client("CAP END")
        .welcome(":soju.test 001 papo :Welcome to soju, papo")
        .server(":soju.test 422 papo :No MOTD")
        .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn raw_post_registration_sasl_abort_is_a_nonfatal_warning() {
    registered_with_required_sasl()
        .send(papo_proto::Line::new("AUTHENTICATE").param("PLAIN"))
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .event(|event| {
            matches!(event, Event::Line(message)
                if message.as_ref().command_is("AUTHENTICATE"))
        })
        .send(papo_proto::Line::new("AUTHENTICATE").param("*"))
        .client("AUTHENTICATE *")
        .server(":soju.test 906 papo :SASL authentication aborted")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason == "SASL authentication aborted")
        })
        .inspect(|session| assert_eq!(session.account(), Some("alice")))
        .no_client()
        .no_event();
}

#[test]
fn late_sasl_statuses_do_not_change_the_managed_exchange() {
    for (numeric, reason) in [
        ("902", "You must use a nick assigned to you"),
        ("904", "SASL authentication failed"),
        ("905", "SASL message too long"),
    ] {
        registered_with_required_sasl()
            .server(&format!(":soju.test {numeric} papo :{reason}"))
            .event(|event| {
                matches!(event, Event::AuthenticationFailed { reason: actual, .. }
                    if actual == reason)
            })
            .inspect(|session| assert_eq!(session.account(), Some("alice")))
            .no_client()
            .no_event();
    }

    for (line, numeric) in [
        (
            ":soju.test 907 papo :You have already authenticated using SASL",
            "907",
        ),
        (
            ":soju.test 908 papo EXTERNAL,PLAIN :are available SASL mechanisms",
            "908",
        ),
    ] {
        registered_with_required_sasl()
            .server(line)
            .event(|event| {
                matches!(event, Event::Line(message)
                    if message.as_ref().command_is(numeric))
            })
            .inspect(|session| assert_eq!(session.account(), Some("alice")))
            .no_client()
            .no_event();
    }
}

#[test]
fn a_failed_reauthentication_fails_only_that_call() {
    for (failure, reason) in [
        (
            ":soju.test 904 papo :SASL authentication failed: Account does not exist",
            "SASL authentication failed: Account does not exist",
        ),
        (
            ":soju.test 902 papo :You must use a nick assigned to you",
            "You must use a nick assigned to you",
        ),
    ] {
        registered_with_required_sasl()
            .reauthenticate()
            .client("AUTHENTICATE PLAIN")
            .server(failure)
            .event(|event| {
                matches!(event, Event::AuthenticationFailed { reason: actual, .. }
                    if actual.contains(reason))
            })
            .inspect(|session| assert_eq!(session.account(), Some("alice")))
            .no_client()
            .no_event();
    }
}

#[test]
fn reauthentication_without_sasl_fails_only_that_call() {
    registered_with_required_sasl()
        .server(":soju.test CAP papo DEL sasl")
        .event(|event| matches!(event, Event::CapsChanged { .. }))
        .event(|event| matches!(event, Event::FeaturesChanged { .. }))
        .reauthenticate()
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("does not currently advertise sasl"))
        })
        .no_client()
        .no_event()
        .disconnect()
        .connect_default()
        .server(":soju.test CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server(":soju.test CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":soju.test 904 papo :SASL authentication failed")
        .event(|event| {
            matches!(
                event,
                Event::Close {
                    kind: CloseKind::SaslFailed,
                    ..
                }
            )
        });
}

#[test]
fn in_exchange_sasl_failure_still_closes_required_authentication() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":soju.test 904 papo :SASL authentication failed")
    .event(|event| {
        matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
            if reason.contains("SASL authentication failed"))
    })
    .no_client()
    .no_event();
}

fn sasl_offered_after_connect() -> Transcript {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2")).optional()),
    )
    .connect_default()
    .server("CAP * LS :")
    .client("CAP END")
    .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
    .welcome(":ergo.test 001 papo :Welcome")
    .server(":ergo.test 376 papo :End of MOTD")
    .event(|event| matches!(event, Event::Connected(_)))
    .server("CAP papo NEW :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP papo ACK :sasl")
    .event(|event| {
        matches!(event, Event::CapsChanged { added, .. }
            if added.contains_key(&cap::SASL))
    })
    .event(|event| {
        matches!(event, Event::FeaturesChanged { added, .. }
                if added.contains(&papo_core::Feature::Sasl))
    })
    .no_client()
}

#[test]
fn authenticates_after_connect_only_when_asked() {
    sasl_offered_after_connect()
        .reauthenticate()
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
        .server(":ergo.test 900 papo papo!u@host alice :logged in")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .inspect(|session| assert_eq!(session.account(), Some("alice")));
}

#[test]
fn aborts_reauthentication_when_sasl_disappears_and_accepts_907() {
    sasl_offered_after_connect()
        .reauthenticate()
        .client("AUTHENTICATE PLAIN")
        .server("CAP * DEL :sasl")
        .event(|event| {
            matches!(event, Event::CapsChanged { removed, .. }
                if removed.contains_key(&cap::SASL))
        })
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("disappeared"))
        })
        .event(|event| {
            matches!(event, Event::FeaturesChanged { removed, .. }
                    if removed.contains(&papo_core::Feature::Sasl))
        })
        .deadline(Some(std::time::Duration::from_secs(60)))
        .server("CAP * NEW :sasl")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, .. }
                if added.contains_key(&cap::SASL))
        })
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, .. }
                    if added.contains(&papo_core::Feature::Sasl))
        })
        .no_client()
        .reauthenticate()
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 907 papo :already authenticated")
        .no_client()
        .reauthenticate()
        .client("AUTHENTICATE PLAIN");
}

#[test]
fn required_sasl_distinguishes_unavailable_from_failed_login() {
    for transcript in [
        Transcript::new(
            Config::default()
                .sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
        )
        .connect_default()
        .server("CAP * LS :")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::SaslUnavailable, reason, .. } if reason.contains("did not advertise"))),
        Transcript::new(
            Config::default()
                .sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
        )
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * NAK :sasl")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::SaslUnavailable, reason, .. } if reason.contains("rejected"))),
    ] {
        transcript.no_client();
    }

    Transcript::new(Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 904 alice :SASL authentication failed")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
                if reason == "PLAIN: SASL authentication failed; every configured SASL mechanism failed")
        })
        .no_client();
}

#[test]
fn required_sasl_timeout_is_unavailable_after_the_abort_deadline() {
    Transcript::new(
        Config::default()
            .welcome_timeout(std::time::Duration::from_secs(90))
            .sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .advance(std::time::Duration::from_secs(30))
    .client("AUTHENTICATE *")
    .advance(std::time::Duration::from_secs(30))
    .event(|event| {
        matches!(event, Event::Close { kind: CloseKind::SaslUnavailable, reason, .. }
                if reason.contains("timed out") && reason.contains("did not acknowledge"))
    })
    .no_client();
}

#[test]
fn terminal_sasl_numerics_do_not_try_another_mechanism() {
    for numeric in ["902", "906"] {
        Transcript::new(secure_sasl(
            Sasl::password("alice", papo_core::Secret::new("hunter2")).optional(),
        ))
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
            .sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN standard-replies")
    .client("CAP REQ :standard-replies sasl")
    .server("CAP * ACK :standard-replies sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":ergo.test NOTE AUTHENTICATE VERIFICATION_REQUIRED alice :Verify your account")
    .event(|event| matches!(event, Event::StandardReply(reply) if reply.code == "VERIFICATION_REQUIRED"))
    .server(":ergo.test FAIL * ACCOUNT_REQUIRED :Authentication required")
    .event(|event| matches!(event, Event::Close { kind: CloseKind::AccountRequired, reason, .. } if reason == "Authentication required"))
    .event(|event| matches!(event, Event::StandardReply(reply) if reply.code == "ACCOUNT_REQUIRED"))
    .no_client();
}

#[test]
fn informational_account_required_cap_closes_without_credentials() {
    Transcript::new(Config::default())
        .connect_default()
        .server("CAP * LS :soju.im/account-required sasl=PLAIN standard-replies")
        .event(|event| {
            matches!(event, Event::Close { kind: CloseKind::AccountRequired, reason, .. }
            if reason.contains("requires an authenticated account"))
        })
        .no_client();
}

#[test]
fn informational_account_required_cap_is_not_requested_with_credentials() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :soju.im/account-required sasl=PLAIN standard-replies")
    .client("CAP REQ :standard-replies sasl")
    .no_event();
}

#[test]
fn informational_account_required_cap_accepts_a_server_password() {
    Transcript::new(Config::default().password(papo_core::Secret::new("mC8r-Zq4v_N7xLp2")))
        .connect()
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server("CAP * LS :soju.im/account-required standard-replies")
        .client("CAP REQ :standard-replies")
        .client("PASS mC8r-Zq4v_N7xLp2")
        .server("CAP * ACK :standard-replies")
        .client("CAP END")
        .welcome(":soju.test 001 papo :Welcome to soju, papo")
        .server(":soju.test 422 papo :No MOTD")
        .event(|event| matches!(event, Event::Connected(_)));
}

#[test]
fn plaintext_connections_refuse_oauth_credentials() {
    let config = Config::default().sasl(Sasl::oauth_bearer(papo_core::Secret::new(
        "oauth-token-7391",
    )));
    Transcript::rate_limited(config)
        .connect()
        .event(|event| matches!(event, Event::Close { reason, .. } if reason.contains("plaintext")))
        .no_client();
}

#[test]
fn unrelated_fail_replies_do_not_trigger_account_required_policy() {
    for reply in [
        ":ergo.test FAIL AUTHENTICATE ACCOUNT_REQUIRED :Authentication required",
        ":ergo.test FAIL * OTHER_CODE :A different failure",
    ] {
        Transcript::new(
            Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
        )
        .connect_default()
        .server("CAP * LS :sasl=PLAIN standard-replies")
        .client("CAP REQ :standard-replies sasl")
        .server("CAP * ACK :standard-replies sasl")
        .client("AUTHENTICATE PLAIN")
        .server(reply)
        .event(|event| matches!(event, Event::StandardReply(_)))
        .no_client()
        .deadline(Some(std::time::Duration::from_secs(30)));
    }
}

#[test]
fn aborts_at_the_literal_30_second_deadline_then_falls_back() {
    Transcript::with_entropy(
        secure_sasl(Sasl::password("alice", papo_core::Secret::new("hunter2"))),
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
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
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
        secure_sasl(Sasl::password("alice", papo_core::Secret::new("hunter2"))),
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

    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":ergo.test 900 alice alice!~u@host alice :You are now logged in as alice")
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END")
    .welcome(":ergo.test 001 alice :Welcome to the ErgoTest IRC Network alice")
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
fn authenticates_with_empty_external_authorization_identity() {
    Transcript::new(Config::default().sasl(Sasl::external()))
        .connect_default()
        .server("CAP * LS :sasl=EXTERNAL,PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE EXTERNAL")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE +")
        .server(":ergo.test 903 alice :SASL authentication successful")
        .client("CAP END");
}

fn scram_start(require_sasl: bool) -> Transcript {
    let sasl = Sasl::scram_sha256("user", papo_core::Secret::new("pencil"));
    let sasl = if require_sasl { sasl } else { sasl.optional() };
    Transcript::with_entropy(Config::default().sasl(sasl), Box::new(FixedEntropy))
        .connect_default()
        .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE SCRAM-SHA-256")
}

fn scram_after_client_first(transcript: Transcript) -> Transcript {
    transcript
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE biwsbj11c2VyLHI9V2xwYVdscGFXbHBhV2xwYVdscGFXbHBh")
}

fn scram_after_client_final(transcript: Transcript) -> Transcript {
    scram_after_client_first(transcript)
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
    scram_after_client_final(scram_start(false)).server(server_final)
}

#[test]
fn each_scram_step_renews_the_deadline() {
    scram_after_client_first(scram_start(false)).deadline(Some(std::time::Duration::from_secs(30)));
    scram_after_client_final(scram_start(false)).deadline(Some(std::time::Duration::from_secs(30)));
}

#[test]
fn rejects_a_non_empty_initial_scram_challenge() {
    scram_start(false)
        .server("AUTHENTICATE YQ==")
        .client("AUTHENTICATE *")
        .server(":ergo.test 906 user :SASL authentication aborted")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("unexpected SCRAM challenge"))
        })
        .client("CAP END");
}

#[test]
fn rejects_scram_success_without_a_verified_server_proof() {
    for numeric in [903, 907] {
        for transcript in [
            scram_start(true),
            scram_after_client_final(scram_start(true)),
        ] {
            transcript
                .server(&format!(
                    ":ergo.test {numeric} user :SASL authentication successful"
                ))
                .event(|event| {
                    matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
                        if reason.contains("without a verified server proof"))
                })
                .no_client();
        }

        for transcript in [
            scram_start(false),
            scram_after_client_final(scram_start(false)),
        ] {
            transcript
                .server(&format!(
                    ":ergo.test {numeric} user :SASL authentication successful"
                ))
                .event(|event| {
                    matches!(event, Event::AuthenticationFailed { reason, .. }
                        if reason.contains("without a verified server proof"))
                })
                .client("CAP END");
        }
    }
}

#[test]
fn unverified_scram_success_discards_the_pending_account() {
    scram_after_client_final(scram_start(false))
        .server(":ergo.test 900 user user!~u@host user :You are now logged in as user")
        .inspect(|session| {
            assert_eq!(session.account(), None);
            assert!(matches!(
                session.state().account(),
                AccountStatus::Unknown | AccountStatus::LoggedOut
            ));
        })
        .server(":ergo.test 903 user :SASL authentication successful")
        .event(|event| {
            matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("without a verified server proof"))
        })
        .client("CAP END")
        .inspect(|session| {
            assert_eq!(session.account(), None);
            assert!(matches!(
                session.state().account(),
                AccountStatus::Unknown | AccountStatus::LoggedOut
            ));
        });
}

#[test]
fn unverified_scram_success_does_not_publish_a_late_account() {
    let logged_in = ":ergo.test 900 papo papo!~u@host user :You are now logged in as user";
    let after_rejected_success = || {
        scram_start(false)
            .server(":ergo.test 903 papo :SASL authentication successful")
            .event(|event| {
                matches!(event, Event::AuthenticationFailed { reason, .. }
                    if reason.contains("without a verified server proof"))
            })
            .client("CAP END")
            .server(logged_in)
            .inspect(|session| assert_eq!(session.account(), None))
    };

    let retry = after_rejected_success()
        .welcome(":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .reauthenticate()
        .client("AUTHENTICATE SCRAM-SHA-256");
    scram_after_client_final(retry)
        .server("AUTHENTICATE dj1hV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==")
        .client("AUTHENTICATE +")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .server(logged_in)
        .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("900")))
        .inspect(|session| assert_eq!(session.account(), Some("user")));

    after_rejected_success()
        .disconnect()
        .connect_default()
        .server(logged_in)
        .inspect(|session| assert_eq!(session.account(), Some("user")));
}

#[test]
fn sasl_failure_discards_the_pending_account() {
    Transcript::new(
        Config::default().sasl(
            Sasl::plain(
                "alice",
                papo_core::Secret::new("correct horse battery staple"),
            )
            .optional(),
        ),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server(":solanum.example 900 alice alice!user@host alice :logged in")
    .inspect(|session| assert_eq!(session.account(), None))
    .server(":solanum.example 904 alice :SASL authentication failed")
    .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert!(matches!(
            session.state().account(),
            AccountStatus::Unknown | AccountStatus::LoggedOut
        ));
    });
}

#[test]
fn rejects_scram_success_while_aborting_a_bad_server_proof() {
    for require_sasl in [true, false] {
        let transcript = scram_after_client_final(scram_start(require_sasl))
            .server("AUTHENTICATE dj1iV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==")
            .client("AUTHENTICATE *")
            .server(":ergo.test 900 user user!u@host user :You are now logged in as user")
            .server(":ergo.test 903 user :SASL authentication successful");
        if require_sasl {
            transcript
                .event(|event| {
                    matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
                        if reason.contains("ServerSignatureMismatch"))
                })
                .no_client();
        } else {
            transcript
                .event(|event| {
                    matches!(event, Event::AuthenticationFailed { reason, .. }
                        if reason.contains("ServerSignatureMismatch"))
                })
                .client("CAP END");
        }
    }
}

#[test]
fn does_not_accept_sasl_success_before_capability_ack() {
    for numeric in [903, 907] {
        for require_sasl in [true, false] {
            let sasl = Sasl::plain("alice", papo_core::Secret::new("hunter2"));
            let sasl = if require_sasl { sasl } else { sasl.optional() };
            let transcript = Transcript::new(Config::default().sasl(sasl))
                .connect_default()
                .server("CAP * LS :sasl=PLAIN")
                .client("CAP REQ :sasl")
                .server(&format!(
                    ":ergo.test {numeric} alice :SASL authentication successful"
                ));
            let transcript = if numeric == 907 {
                transcript.event(|event| {
                    matches!(event, Event::Line(message) if message.as_ref().command_is("907"))
                })
            } else {
                transcript
            };
            let transcript = transcript.no_client().server("CAP * NAK :sasl");
            if require_sasl {
                transcript
                    .event(|event| {
                        matches!(event, Event::Close { kind: CloseKind::SaslUnavailable, reason, .. }
                            if reason.contains("rejected the sasl capability"))
                    })
                    .no_client();
            } else {
                transcript
                    .event(|event| {
                        matches!(event, Event::AuthenticationFailed { reason, .. }
                            if reason.contains("rejected the sasl capability"))
                    })
                    .client("CAP END");
            }
        }
    }
}

#[test]
fn verified_scram_success_publishes_the_pending_account() {
    scram_until_server_final(
        "AUTHENTICATE dj1hV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==",
    )
    .client("AUTHENTICATE +")
    .server(":ergo.test 900 user user!~u@host user :You are now logged in as user")
    .inspect(|session| {
        assert_eq!(session.account(), None);
        assert!(matches!(
            session.state().account(),
            AccountStatus::Unknown | AccountStatus::LoggedOut
        ));
    })
    .server(":ergo.test 903 user :SASL authentication successful")
    .client("CAP END")
    .inspect(|session| {
        assert_eq!(session.account(), Some("user"));
        assert!(
            matches!(session.state().account(), AccountStatus::LoggedIn(name) if name == "user")
        );
    });
}

#[test]
fn rejects_a_tampered_scram_server_signature() {
    scram_until_server_final(
        "AUTHENTICATE dj1iV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==",
    )
    .client("AUTHENTICATE *")
    .server(":ergo.test 906 user :SASL authentication aborted")
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason, .. }
            if reason.contains("ServerSignatureMismatch"))
    })
    .client("CAP END");
}

#[test]
fn rejects_an_excessive_scram_iteration_count_before_derivation() {
    Transcript::with_entropy(
        Config::default().sasl(Sasl::scram_sha256("user", papo_core::Secret::new("pencil"))),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsbj11c2VyLHI9V2xwYVdscGFXbHBhV2xwYVdscGFXbHBh")
    .server(concat!(
        "AUTHENTICATE ",
        "cj1XbHBhV2xwYVdscGFXbHBhV2xwYVdscGEtdG9vLW1hbnkscz1XMjJaYUowU05ZN3NvRXNVRWpi",
        "NmdRPT0saT00Mjk0OTY3Mjk1"
    ))
    .client("AUTHENTICATE *")
    .server(":ergo.test 906 user :SASL authentication aborted")
    .event(|event| {
        matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
            if reason.contains("IterationCountTooHigh"))
    })
    .no_client();
}

#[test]
fn a_bad_scram_proof_never_falls_back_to_plain() {
    Transcript::with_entropy(
        Config::default().sasl(Sasl::password("user", papo_core::Secret::new("pencil"))),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE SCRAM-SHA-256")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE biwsbj11c2VyLHI9V2xwYVdscGFXbHBhV2xwYVdscGFXbHBh")
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
    .server("AUTHENTICATE dj1iV3UvY3hvOUIyWko1ajMxdnRZeTlObTAxUGY4U2FmbXpGQjN3VEgrS2l3PQ==")
    .client("AUTHENTICATE *")
    .server(":ergo.test 906 user :SASL authentication aborted")
    .event(|event| {
        matches!(event, Event::Close { kind: CloseKind::SaslFailed, reason, .. }
            if reason.contains("ServerSignatureMismatch"))
    })
    .no_client();
}

#[test]
fn replays_recorded_ergo_plain_login() {
    Transcript::new(
        Config::default()
            .wanted_caps(Vec::<String>::new())
            .sasl(Sasl::plain(
                "paposasl",
                papo_core::Secret::new("v3ry-Long_Unique-Password-7391"),
            )),
    )
    .connect_default()
    .load("tests/data/ergo-sasl-plain.irc")
    .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("001")))
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
                papo_core::Secret::new("v3ry-Long_Unique-Password-7391"),
            )),
        Box::new(FixedEntropy),
    )
    .connect_default()
    .load("tests/data/ergo-sasl-scram.irc")
    .event(|event| matches!(event, Event::Line(message) if message.as_ref().command_is("001")))
    .inspect(|session| {
        assert_eq!(session.account(), Some("paposasl"));
        assert_eq!(session.nick(), "paposasl");
    });
}

#[test]
fn rejects_unoffered_and_908_removed_mechanisms() {
    Transcript::new(Config::default().sasl(Sasl::external().optional()))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .event(|event| matches!(event, Event::AuthenticationFailed { .. }))
        .client("CAP END");

    Transcript::with_entropy(
        Config::default()
            .sasl(Sasl::password("alice", papo_core::Secret::new("hunter2")).optional()),
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
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
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
fn a_partial_server_challenge_renews_the_deadline() {
    let first = "YWFh".repeat(100);
    assert_eq!(first.len(), 400);
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2"))),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .advance(std::time::Duration::from_secs(17))
    .server(&format!("AUTHENTICATE {first}"))
    .deadline(Some(std::time::Duration::from_secs(30)))
    .advance(std::time::Duration::from_secs(29))
    .no_client()
    .server("AUTHENTICATE YQ==")
    .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
    .server(":ergo.test 903 alice :SASL authentication successful")
    .client("CAP END");
}

#[test]
fn aborts_a_challenge_without_a_payload() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2")).optional()),
    )
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
fn fails_when_the_server_does_not_acknowledge_an_abort() {
    Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2")).optional()),
    )
    .connect_default()
    .server("CAP * LS :sasl=PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE")
    .client("AUTHENTICATE *")
    .deadline(Some(std::time::Duration::from_secs(30)))
    .advance(std::time::Duration::from_secs(30))
    .event(|event| {
        matches!(event, Event::AuthenticationFailed { reason, .. }
                if reason.contains("did not acknowledge"))
    })
    .client("CAP END");
}

#[test]
fn aborts_a_server_challenge_over_4096_bytes() {
    let chunk = format!("AUTHENTICATE {}", "YWFh".repeat(100));
    let mut transcript = Transcript::new(
        Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("hunter2")).optional()),
    )
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

#[test]
fn login_refusals_redact_decoded_credentials() {
    let credential = "login-credential-4821";
    for (mechanism, sasl, response) in [
        (
            "PLAIN",
            Sasl::plain("alice", papo_core::Secret::new(credential)),
            format!("\0alice\0{credential}"),
        ),
        (
            "OAUTHBEARER",
            Sasl::oauth_bearer(papo_core::Secret::new(credential)),
            format!("n,,\x01auth=Bearer {credential}\x01\x01"),
        ),
    ] {
        Transcript::new(Config::default().sasl(sasl))
            .connect_default()
            .server(&format!(":ergo.test CAP * LS :sasl={mechanism}"))
            .client("CAP REQ :sasl")
            .server(":ergo.test CAP * ACK :sasl")
            .client(&format!("AUTHENTICATE {mechanism}"))
            .server("AUTHENTICATE +")
            .client(&format!("AUTHENTICATE {}", STANDARD.encode(response)))
            .server(&format!(
                ":ergo.test 904 papo :SASL authentication failed: {credential}"
            ))
            .event_until(|event| {
                let Event::Close { reason, .. } = event else {
                    return false;
                };
                assert!(reason.contains("<redacted>"));
                assert!(!reason.contains(credential));
                true
            });
    }
}

#[test]
fn sasl_payload_over_the_credential_limit_aborts_the_exchange() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", papo_core::Secret::new("account-key-4821".repeat(40))).optional()))
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * ACK :sasl")
        .client("AUTHENTICATE PLAIN")
        .server(":ergo.test 904 papo :SASL authentication failed")
        .client("CAP END")
        .server(":ergo.test 001 papo :Welcome to the Ergo Test Network")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .event_until(|event| matches!(event, Event::Connected(_)))
        .update(|session| {
            let now = std::time::Instant::now();
            for index in 0..256 {
                session.send(now, papo_proto::Line::new("JOIN").param("#staff").param(papo_core::Secret::new(format!("staff-key-{index:03}")))).unwrap();
                assert!(session.poll_transmit().is_some());
            }
        })
        .reauthenticate()
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .server("PING :before-sasl-abort")
        .client("PONG before-sasl-abort")
        .client("AUTHENTICATE *")
        .no_client()
        .event_until(|event| matches!(event, Event::StateLimitReached { limit: papo_core::StateLimit::Credentials, .. }))
        .server(":ergo.test 906 papo :SASL authentication aborted")
        .event_until(|event| matches!(event, Event::AuthenticationFailed { reason, .. } if reason.contains("credential tracking limit reached")))
        .server("PING :ergo.test")
        .client("PONG ergo.test")
        .ignore_rest();
}
