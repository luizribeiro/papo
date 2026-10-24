//! Transcript tests for `draft/account-registration`.

mod support;

use papo_core::{Config, Event, Feature, Refusal, RequestError, ResponseMatcher, Secret};
use papo_proto::Line;
use support::{Transcript, assert_session_debug_redacts, queue_debug_marker, unsupported_error};

fn registered() -> Transcript {
    Transcript::new(
        Config::default().wanted_caps(["draft/account-registration", "labeled-response"]),
    )
    .connect_default()
    .server("CAP * LS :draft/account-registration=custom-account-name labeled-response")
    .client("CAP REQ :draft/account-registration labeled-response")
    .server("CAP * ACK :draft/account-registration labeled-response")
    .client("CAP END")
    .welcome(":ergo.test 001 tester :Welcome to Ergo")
    .server(":ergo.test 422 tester :MOTD File is missing")
    .event(|event| matches!(event, Event::Connected(_)))
    .server(":ergo.test 005 tester CASEMAPPING=ascii :are supported")
    .event(|event| matches!(event, Event::CasemappingChanged { .. }))
}

fn early_registration_waiting() -> Transcript {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            session
                .create_account(std::time::Instant::now(), None, None, "password")
                .unwrap();
        })
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER * * password")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
}

#[test]
fn configured_registration_waits_for_before_connect_capability() {
    let config = Config::default()
        .wanted_caps(["draft/account-registration"])
        .create_account_on_connect(
            Some("alice"),
            Some("alice@example.org"),
            "unique-registration-password",
        );
    assert!(!format!("{config:?}").contains("unique-registration-password"));
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect,custom-account-name")
        .client("CAP REQ :draft/account-registration")
        .server("CAP * ACK :draft/account-registration")
        .update(queue_debug_marker)
        .inspect(|session| assert_session_debug_redacts(session, "unique-registration-password"))
        .client("REGISTER alice alice@example.org unique-registration-password")
        .client("PRIVMSG #papo :ordinary debug marker")
        .event(|event| matches!(event, Event::RequestUntracked { .. }));
}

#[test]
fn secure_connections_allow_account_registration_without_plaintext_opt_in() {
    Transcript::rate_limited(
        Config::default()
            .connection_security(papo_core::sts::ConnectionSecurity::Secure)
            .wanted_caps(["draft/account-registration"]),
    )
    .connect_default()
    .server("CAP * LS :draft/account-registration=before-connect")
    .client("CAP REQ :draft/account-registration")
    .update(|session| {
        session
            .create_account(
                std::time::Instant::now(),
                None,
                None,
                "correct horse battery staple",
            )
            .unwrap();
    })
    .server("CAP * ACK :draft/account-registration")
    .client("REGISTER * * :correct horse battery staple")
    .event(|event| matches!(event, Event::RequestUntracked { .. }));
}

#[test]
fn configured_registration_survives_a_multiline_capability_listing() {
    let config = Config::default()
        .wanted_caps(["draft/account-registration"])
        .create_account_on_connect(
            Some("alice"),
            None::<String>,
            "unique-registration-password",
        );
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS * :draft/account-registration=before-connect,custom-account-name")
        .no_client()
        .server("CAP * LS :message-tags")
        .client("CAP REQ :draft/account-registration")
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER alice * unique-registration-password")
        .event(|event| matches!(event, Event::RequestUntracked { .. }));
}

#[test]
fn configured_registration_is_sent_only_once() {
    let config = Config::default()
        .wanted_caps(["draft/account-registration"])
        .create_account_on_connect(Some("alice"), None::<String>, "account-password-7391");
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect,custom-account-name")
        .client("CAP REQ :draft/account-registration")
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER alice * account-password-7391")
        .server(":ergo.test REGISTER SUCCESS alice :Account successfully registered")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::AccountCreated { account, .. } if account == "alice"))
        .client("CAP END")
        .disconnect()
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect,custom-account-name")
        .client("CAP REQ :draft/account-registration")
        .server("CAP * ACK :draft/account-registration")
        .client("CAP END")
        .no_client();
}

#[test]
fn registers_an_account_with_a_labelled_response() {
    registered()
        .update(|session| {
            session
                .create_account(
                    std::time::Instant::now(),
                    Some("alice"),
                    Some("alice@example.org"),
                    "correct horse battery staple",
                )
                .unwrap();
        })
        .client("@label=0 REGISTER alice alice@example.org :correct horse battery staple")
        .server("@label=0 :ergo.test REGISTER SUCCESS alice :Account successfully registered")
        .event(|event| {
            matches!(event, Event::AccountCreated { account, logged_in: true, .. } if account == "alice" && account.casemapping() == papo_proto::Casemapping::Ascii)
        })
        .event(|event| matches!(event, Event::Response(_)))
        .inspect(|session| assert_eq!(session.account(), Some("alice")));
}

#[test]
fn rejects_a_byte_account_password() {
    registered()
        .update(|session| {
            assert_eq!(
                session.create_account(
                    std::time::Instant::now(),
                    Some("alice"),
                    Some("alice@example.org"),
                    Secret::from_bytes([0xff]),
                ),
                Err(RequestError::InvalidSecret)
            );
        })
        .no_client();
}

#[test]
fn verifies_an_account_after_the_server_requires_it() {
    registered()
        .update(|session| {
            session
                .create_account(
                    std::time::Instant::now(),
                    Some("alice"),
                    Some("alice@example.org"),
                    "correct horse battery staple",
                )
                .unwrap();
        })
        .client("@label=0 REGISTER alice alice@example.org :correct horse battery staple")
        .server(
            "@label=0 :ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent",
        )
        .event(|event| {
            matches!(event, Event::VerificationRequired { account, message, .. }
                if account == "alice" && account.casemapping() == papo_proto::Casemapping::Ascii && message == "Verification code sent")
        })
        .event(|event| matches!(event, Event::Response(_)))
        .update(|session| {
            session
                .verify(std::time::Instant::now(), "alice", "39gvcdg4myvnmdcfhvd6exsv4n")
                .unwrap();
            queue_debug_marker(session);
        })
        .inspect(|session| assert_session_debug_redacts(session, "39gvcdg4myvnmdcfhvd6exsv4n"))
        .client("@label=1 VERIFY alice 39gvcdg4myvnmdcfhvd6exsv4n")
        .client("PRIVMSG #papo :ordinary debug marker")
        .server("@label=1 :ergo.test VERIFY SUCCESS alice :Account successfully registered")
        .event(|event| {
            matches!(event, Event::AccountCreated { account, logged_in: true, .. } if account == "alice")
        })
        .event(|event| matches!(event, Event::Response(_)));
}

#[test]
fn rejects_a_byte_verification_code() {
    registered()
        .update(|session| {
            assert_eq!(
                session.verify(
                    std::time::Instant::now(),
                    "alice",
                    Secret::from_bytes([0xff]),
                ),
                Err(RequestError::InvalidSecret)
            );
        })
        .no_client();
}

#[test]
fn types_every_registered_account_failure_code() {
    use Refusal as E;

    let cases = [
        (
            "ACCOUNT_EXISTS",
            E::AccountExists,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "BAD_ACCOUNT_NAME",
            E::BadAccountName,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "ACCOUNT_NAME_MUST_BE_NICK",
            E::AccountNameMustMatchNick,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "NEED_NICK",
            E::NicknameRequired,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "ALREADY_AUTHENTICATED",
            E::AlreadyAuthenticated,
            &["REGISTER", "VERIFY"][..],
            "PERSISTENCE",
        ),
        (
            "WEAK_PASSWORD",
            E::WeakPassword,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "UNACCEPTABLE_PASSWORD",
            E::UnacceptablePassword,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "INVALID_EMAIL",
            E::InvalidEmail,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "UNACCEPTABLE_EMAIL",
            E::UnacceptableEmail,
            &["REGISTER"][..],
            "VERIFY",
        ),
        (
            "COMPLETE_CONNECTION_REQUIRED",
            E::NotConnectedYet,
            &["REGISTER", "VERIFY"][..],
            "PERSISTENCE",
        ),
        (
            "INVALID_CODE",
            E::InvalidVerificationCode,
            &["VERIFY"][..],
            "REGISTER",
        ),
        (
            "TEMPORARILY_UNAVAILABLE",
            E::TemporarilyUnavailable,
            &["REGISTER", "VERIFY"][..],
            "PERSISTENCE",
        ),
    ];
    for (code, expected, valid_commands, invalid_command) in cases {
        for command in valid_commands {
            registered()
                .server(&format!(
                    ":ergo.test FAIL {command} {code} alice :Account request rejected"
                ))
                .event(|event| {
                    matches!(event, Event::StandardReply(reply)
                        if reply.refusal() == Some(expected))
                });
        }
        registered()
            .server(&format!(
                ":ergo.test FAIL {invalid_command} {code} alice :Account request rejected"
            ))
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                    if reply.command == invalid_command
                        && reply.code == code
                        && reply.refusal() == Some(Refusal::Other))
            });
    }
}

#[test]
fn types_a_labelled_verification_failure() {
    registered()
        .update(|session| {
            session
                .verify(std::time::Instant::now(), "alice", "expired-code")
                .unwrap();
        })
        .client("@label=0 VERIFY alice expired-code")
        .server("@label=0 :ergo.test FAIL VERIFY INVALID_CODE alice :Invalid verification code")
        .event(|event| {
            matches!(event, Event::Response(response)
                if response.refusal() == Some(Refusal::InvalidVerificationCode))
        });
}

#[test]
fn enforces_account_registration_capability_values_locally() {
    let config = Config::default().wanted_caps(["draft/account-registration"]);
    Transcript::new(config)
        .connect_default()
        .server("CAP * LS :draft/account-registration=email-required")
        .client("CAP REQ :draft/account-registration")
        .server("CAP * ACK :draft/account-registration")
        .client("CAP END")
        .welcome(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)))
        .update(|session| {
            assert_eq!(
                session.create_account(std::time::Instant::now(), None, None, "hunter2"),
                Err(papo_core::RequestError::EmailRequired)
            );
            assert_eq!(
                session.create_account(
                    std::time::Instant::now(),
                    Some("other-account"),
                    Some("alice@example.org"),
                    "hunter2",
                ),
                Err(papo_core::RequestError::CustomAccountNameUnsupported)
            );
        })
        .no_client();
}

#[test]
fn registers_before_connect_and_holds_cap_end_until_success() {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect,email-required")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            session
                .create_account(
                    std::time::Instant::now(),
                    None,
                    Some("alice@example.org"),
                    "correct horse battery staple",
                )
                .unwrap();
            assert!(!format!("{session:?}").contains("correct horse battery staple"));
        })
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER * alice@example.org :correct horse battery staple")
        .no_client()
        .server(":ergo.test REGISTER SUCCESS alice :Account successfully registered")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::AccountCreated { account, .. } if account == "alice"))
        .client("CAP END");
}

#[test]
fn rejects_early_registration_without_before_connect() {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=email-required")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            assert_eq!(
                session.create_account(
                    std::time::Instant::now(),
                    None,
                    Some("alice@example.org"),
                    "hunter2",
                ),
                Err(unsupported_error(Feature::AccountCreation))
            );
        })
        .server("CAP * ACK :draft/account-registration")
        .client("CAP END")
        .update(|session| {
            assert_eq!(
                session.verify(std::time::Instant::now(), "alice", "pending-code"),
                Err(unsupported_error(Feature::AccountCreation))
            );
        })
        .no_client();
}

#[test]
fn early_verification_keeps_cap_negotiation_open() {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect,email-required")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            session
                .create_account(
                    std::time::Instant::now(),
                    None,
                    Some("alice@example.org"),
                    "correct horse battery staple",
                )
                .unwrap();
        })
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER * alice@example.org :correct horse battery staple")
        .server(":ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::VerificationRequired { account, .. } if account == "alice"))
        .no_client()
        .update(|session| {
            session
                .verify(std::time::Instant::now(), "alice", "39gvcdg4myvnmdcfhvd6exsv4n")
                .unwrap();
        })
        .client("VERIFY alice 39gvcdg4myvnmdcfhvd6exsv4n")
        .server(":ergo.test VERIFY SUCCESS alice :Account successfully registered")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| matches!(event, Event::AccountCreated { account, .. } if account == "alice"))
        .client("CAP END");
}

#[test]
fn early_registration_failure_releases_cap_end() {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            session
                .create_account(std::time::Instant::now(), None, None, "password")
                .unwrap();
        })
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER * * password")
        .server(":ergo.test FAIL REGISTER WEAK_PASSWORD alice :Password is too weak")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.refusal() == Some(Refusal::WeakPassword))
        })
        .client("CAP END");
}

#[test]
fn disconnect_aborts_registration_and_rejects_its_late_success() {
    Transcript::new(
        Config::default().wanted_caps(["draft/account-registration", "labeled-response"]),
    )
    .connect_default()
    .server("CAP * LS :draft/account-registration=before-connect,email-required labeled-response")
    .client("CAP REQ :draft/account-registration labeled-response")
    .update(|session| {
        session
            .create_account(
                std::time::Instant::now(),
                None,
                Some("alice@example.org"),
                "correct horse battery staple",
            )
            .unwrap();
    })
    .server("CAP * ACK :draft/account-registration labeled-response")
    .client("@label=0 REGISTER * alice@example.org :correct horse battery staple")
    .disconnect()
    .event(|event| matches!(event, Event::RequestAborted { .. }))
    .connect_default()
    .server(":ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent")
    .event(|event| matches!(event, Event::Line(_)))
    .server(":ergo.test REGISTER SUCCESS alice :Account successfully registered")
    .event(|event| matches!(event, Event::Line(_)))
    .inspect(|session| assert_eq!(session.account(), None));
}

#[test]
fn verification_challenge_requires_both_register_and_its_result_code() {
    early_registration_waiting()
        .server(":ergo.test VERIFY VERIFICATION_REQUIRED alice :Verification code sent")
        .event(|event| matches!(event, Event::Line(_)))
        .no_client()
        .server(":ergo.test REGISTER CONTINUE alice :Verification code sent")
        .event(|event| matches!(event, Event::Line(_)))
        .no_client()
        .server(":ergo.test REGISTER VERIFICATION_REQUIRED alice :Verification code sent")
        .event(|event| {
            matches!(event, Event::VerificationRequired { account, .. } if account == "alice")
        })
        .no_client();
}

#[test]
fn only_account_request_failures_release_cap_negotiation() {
    early_registration_waiting()
        .server(":ergo.test FAIL PERSISTENCE ACCOUNT_REQUIRED :Log in to use persistence")
        .event(
            |event| matches!(event, Event::StandardReply(reply) if reply.command == "PERSISTENCE"),
        )
        .no_client()
        .server(":ergo.test NOTE REGISTER MAINTENANCE :Registration service maintenance soon")
        .event(|event| matches!(event, Event::StandardReply(reply) if reply.command == "REGISTER"))
        .no_client()
        .server(":ergo.test FAIL REGISTER WEAK_PASSWORD alice :Password is too weak")
        .event(
            |event| matches!(event, Event::StandardReply(reply) if reply.code == "WEAK_PASSWORD"),
        )
        .client("CAP END");
}

#[test]
fn pending_cap_request_preserves_queued_registration() {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            session
                .create_account(std::time::Instant::now(), None, None, "password")
                .unwrap();
        })
        .server("PING :ergo.test")
        .client("PONG ergo.test")
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER * * password")
        .event(|event| matches!(event, Event::RequestUntracked { .. }));
}

#[test]
fn early_registration_reserves_its_id_before_a_later_whois() {
    Transcript::new(
        Config::default().wanted_caps(["draft/account-registration", "labeled-response"]),
    )
    .connect_default()
    .server("CAP * LS :draft/account-registration=before-connect labeled-response")
    .client("CAP REQ :draft/account-registration labeled-response")
    .update(|session| {
        let registration = session.next_request_id();
        session
            .create_account(std::time::Instant::now(), None, None, "unique-password")
            .unwrap();
        assert_ne!(session.next_request_id(), registration);
        let whois = session
            .request_with_matcher(
                std::time::Instant::now(),
                Line::new("WHOIS").with_params(["alice"]),
                ResponseMatcher::Whois("alice".into()),
            )
            .unwrap();
        assert_ne!(whois, registration);
    })
    .client("WHOIS alice")
    .server("CAP * ACK :draft/account-registration labeled-response")
    .client("@label=0 REGISTER * * unique-password");
}

#[test]
fn rejected_cap_request_discards_queued_registration() {
    Transcript::new(Config::default().wanted_caps(["draft/account-registration"]))
        .connect_default()
        .server("CAP * LS :draft/account-registration=before-connect")
        .client("CAP REQ :draft/account-registration")
        .update(|session| {
            session
                .create_account(std::time::Instant::now(), None, None, "password")
                .unwrap();
        })
        .server("CAP * NAK :draft/account-registration")
        .client("CAP END")
        .server("CAP * NEW :draft/account-registration=before-connect")
        .client("CAP REQ :draft/account-registration")
        .server("CAP * ACK :draft/account-registration")
        .event(|event| matches!(event, Event::CapsChanged { .. }))
        .no_client();
}
