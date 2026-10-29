//! Transcript tests for SASL authentication.

mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo_core::{
    Account, CloseKind, Config, Entropy, Event, Sasl, Session, sts::ConnectionSecurity,
};
use support::{Transcript, assert_session_debug_redacts, queue_debug_marker};

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

#[test]
fn configures_mechanisms_policy_and_entropy() {
    let config = Config::default().sasl(
        Sasl::any(
            "alice",
            papo_core::Secret::new("correct horse battery staple"),
        )
        .device("phone"),
    );
    assert!(config.sasl.is_some());

    let session = Session::with_entropy(config, Box::new(FixedEntropy));
    assert!(format!("{session:?}").contains("has_entropy: true"));
}

#[test]
fn debug_output_redacts_server_and_sasl_passwords() {
    let config = Config::default()
        .password(papo_core::Secret::new("server-password-7391"))
        .sasl(Sasl::any(
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
    .client("PRIVMSG #papo :ordinary debug marker");
}

#[test]
fn external_accepts_an_authorization_identity() {
    assert_ne!(Sasl::external(), Sasl::external().authzid("alice"));
    assert_ne!(
        Sasl::plain("alice", papo_core::Secret::new("hunter2")),
        Sasl::plain("alice", papo_core::Secret::new("hunter2")).authzid("admin")
    );
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
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
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
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
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
                if removed.contains_key("sasl"))
        })
        .server("CAP papo NEW :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP papo ACK :sasl")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, .. }
                if added.contains_key("sasl"))
        })
        .inspect(|session| assert_eq!(session.account(), Some("alice")))
        .no_client();
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
fn inactive_authenticate_messages_are_surfaced_unchanged() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
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
    Transcript::new(
        Config::default().sasl(
            Sasl::plain("alice", "correct horse battery staple")
                .device("phone")
                .authzid("admin"),
        ),
    )
    .connect_default()
    .server("CAP * LS :sasl=SCRAM-SHA-256,PLAIN")
    .client("CAP REQ :sasl")
    .server("CAP * ACK :sasl")
    .client("AUTHENTICATE PLAIN")
    .server("AUTHENTICATE +")
    .client("AUTHENTICATE YWRtaW4AYWxpY2VAcGhvbmUAY29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==")
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
        .inspect(|session| {
            assert_eq!(session.account(), Some("emersion"));
            assert!(
                matches!(session.state().account(), Account::LoggedIn(name) if name == "emersion")
            );
        })
        .server(":solanum.example 903 emersion :SASL authentication successful")
        .client("CAP END")
        .server(":solanum.example 901 emersion emersion!user@host :logged out")
        .inspect(|session| {
            assert_eq!(session.account(), None);
            assert_eq!(session.state().account(), &Account::LoggedOut);
        });
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
        secure_sasl(Sasl::any("alice", "hunter2")),
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
            .sasl(Sasl::any("alice", "hunter2")),
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
    Transcript::new(secure_sasl(Sasl::any("alice", "hunter2").optional()))
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
fn authenticates_when_sasl_arrives_after_connect() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2").optional()))
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
        .client("AUTHENTICATE PLAIN")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, .. }
            if added.contains_key("sasl"))
        })
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, .. }
                if added.contains(&papo_core::Feature::Sasl))
        })
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AGFsaWNlAGh1bnRlcjI=")
        .server(":ergo.test 900 papo papo!u@host alice :logged in")
        .server(":ergo.test 903 papo :SASL authentication successful")
        .inspect(|session| assert_eq!(session.account(), Some("alice")));
}

#[test]
fn aborts_reauthentication_when_sasl_disappears_and_accepts_907() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2").optional()))
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
        .client("AUTHENTICATE PLAIN")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, .. }
            if added.contains_key("sasl"))
        })
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, .. }
                if added.contains(&papo_core::Feature::Sasl))
        })
        .server("CAP * DEL :sasl")
        .event(|event| {
            matches!(event, Event::CapsChanged { removed, .. }
            if removed.contains_key("sasl"))
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
        .client("AUTHENTICATE PLAIN")
        .event(|event| {
            matches!(event, Event::CapsChanged { added, .. }
            if added.contains_key("sasl"))
        })
        .event(|event| {
            matches!(event, Event::FeaturesChanged { added, .. }
                if added.contains(&papo_core::Feature::Sasl))
        })
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
                .sasl(Sasl::plain("alice", "hunter2")),
        )
        .connect_default()
        .server("CAP * LS :")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::SaslUnavailable, reason, .. } if reason.contains("did not advertise"))),
        Transcript::new(
            Config::default()
                .sasl(Sasl::plain("alice", "hunter2")),
        )
        .connect_default()
        .server("CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server("CAP * NAK :sasl")
        .event(|event| matches!(event, Event::Close { kind: CloseKind::SaslUnavailable, reason, .. } if reason.contains("rejected"))),
    ] {
        transcript.no_client();
    }

    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
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
            .sasl(Sasl::plain("alice", "hunter2")),
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
        Transcript::new(secure_sasl(Sasl::any("alice", "hunter2").optional()))
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
            .sasl(Sasl::plain("alice", "hunter2")),
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
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
        .connect_default()
        .server("CAP * LS :soju.im/account-required sasl=PLAIN standard-replies")
        .client("CAP REQ :standard-replies sasl")
        .no_event();
}

#[test]
fn informational_account_required_cap_accepts_a_server_password() {
    Transcript::new(Config::default().password("mC8r-Zq4v_N7xLp2"))
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
    let config = Config::default().sasl(Sasl::oauth_bearer("oauth-token-7391"));
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
        Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
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
        secure_sasl(Sasl::any("alice", "hunter2")),
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
        secure_sasl(Sasl::any("alice", "hunter2")),
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
    let sasl = Sasl::scram_sha256("user", "pencil");
    let sasl = if require_sasl { sasl } else { sasl.optional() };
    Transcript::with_entropy(Config::default().sasl(sasl), Box::new(FixedEntropy))
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
fn each_scram_step_renews_the_deadline() {
    scram_after_client_first(false).deadline(Some(std::time::Duration::from_secs(30)));
    scram_after_client_final(false).deadline(Some(std::time::Duration::from_secs(30)));
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
        for transcript in [scram_start(true), scram_after_client_final(true)] {
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

        for transcript in [scram_start(false), scram_after_client_final(false)] {
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
fn rejects_scram_success_while_aborting_a_bad_server_proof() {
    for require_sasl in [true, false] {
        let transcript = scram_after_client_final(require_sasl)
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
fn ignores_sasl_success_before_capability_ack() {
    for numeric in [903, 907] {
        for require_sasl in [true, false] {
            let sasl = Sasl::plain("alice", "hunter2");
            let sasl = if require_sasl { sasl } else { sasl.optional() };
            let transcript = Transcript::new(Config::default().sasl(sasl))
                .connect_default()
                .server("CAP * LS :sasl=PLAIN")
                .client("CAP REQ :sasl")
                .server(&format!(
                    ":ergo.test {numeric} alice :SASL authentication successful"
                ))
                .no_client()
                .server("CAP * NAK :sasl");
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
        matches!(event, Event::AuthenticationFailed { reason, .. }
            if reason.contains("ServerSignatureMismatch"))
    })
    .client("CAP END");
}

#[test]
fn rejects_an_excessive_scram_iteration_count_before_derivation() {
    Transcript::with_entropy(
        Config::default().sasl(Sasl::scram_sha256("user", "pencil")),
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
        Config::default().sasl(Sasl::any("user", "pencil")),
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
            .sasl(Sasl::plain("paposasl", "v3ry-Long_Unique-Password-7391")),
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
                "v3ry-Long_Unique-Password-7391",
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
        Config::default().sasl(Sasl::any("alice", "hunter2").optional()),
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
fn a_partial_server_challenge_renews_the_deadline() {
    let first = "YWFh".repeat(100);
    assert_eq!(first.len(), 400);
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2")))
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
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2").optional()))
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
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2").optional()))
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
    let mut transcript =
        Transcript::new(Config::default().sasl(Sasl::plain("alice", "hunter2").optional()))
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
