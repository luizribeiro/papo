//! Transcript tests for `draft/account-registration`.

mod support;

use papo_core::{AccountRegistrationError, Config, Event};
use support::Transcript;

fn registered() -> Transcript {
    Transcript::new(
        Config::default().wanted_caps(["draft/account-registration", "labeled-response"]),
    )
    .connect_default()
    .server("CAP * LS :draft/account-registration=custom-account-name labeled-response")
    .client("CAP REQ :draft/account-registration labeled-response")
    .server("CAP * ACK :draft/account-registration labeled-response")
    .client("CAP END")
    .server(":ergo.test 001 tester :Welcome to Ergo")
    .server(":ergo.test 422 tester :MOTD File is missing")
    .event(|event| matches!(event, Event::Registered(_)))
}

#[test]
fn registers_an_account_with_a_labelled_response() {
    registered()
        .update(|session| {
            session
                .register(
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
            matches!(event, Event::AccountRegistered { account, logged_in: true } if account == "alice")
        })
        .event(|event| matches!(event, Event::Response(_)))
        .inspect(|session| assert_eq!(session.account(), Some("alice")));
}

#[test]
fn verifies_an_account_after_the_server_requires_it() {
    registered()
        .update(|session| {
            session
                .register(
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
            matches!(event, Event::VerificationRequired { account, message }
                if account == "alice" && message == "Verification code sent")
        })
        .event(|event| matches!(event, Event::Response(_)))
        .update(|session| {
            session
                .verify(std::time::Instant::now(), "alice", "39gvcdg4myvnmdcfhvd6exsv4n")
                .unwrap();
        })
        .client("@label=1 VERIFY alice 39gvcdg4myvnmdcfhvd6exsv4n")
        .server("@label=1 :ergo.test VERIFY SUCCESS alice :Account successfully registered")
        .event(|event| {
            matches!(event, Event::AccountRegistered { account, logged_in: true } if account == "alice")
        })
        .event(|event| matches!(event, Event::Response(_)));
}

#[test]
fn types_every_registered_account_failure_code() {
    use AccountRegistrationError as E;

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
            E::AccountNameMustBeNick,
            &["REGISTER"][..],
            "VERIFY",
        ),
        ("NEED_NICK", E::NeedNick, &["REGISTER"][..], "VERIFY"),
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
            E::CompleteConnectionRequired,
            &["REGISTER", "VERIFY"][..],
            "PERSISTENCE",
        ),
        ("INVALID_CODE", E::InvalidCode, &["VERIFY"][..], "REGISTER"),
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
                        if reply.account_registration_error() == Some(expected))
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
                        && reply.account_registration_error().is_none())
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
                if response.account_registration_error()
                    == Some(AccountRegistrationError::InvalidCode))
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
        .server(":ergo.test 001 alice :Welcome to Ergo")
        .server(":ergo.test 422 alice :MOTD File is missing")
        .event(|event| matches!(event, Event::Registered(_)))
        .update(|session| {
            assert_eq!(
                session.register(std::time::Instant::now(), None, None, "hunter2"),
                Err(papo_core::RequestError::EmailRequired)
            );
            assert_eq!(
                session.register(
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
                .register(
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
        .event(
            |event| matches!(event, Event::AccountRegistered { account, .. } if account == "alice"),
        )
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
                session.register(
                    std::time::Instant::now(),
                    None,
                    Some("alice@example.org"),
                    "hunter2",
                ),
                Err(papo_core::RequestError::Unsupported(
                    "account registration before connection"
                ))
            );
        })
        .server("CAP * ACK :draft/account-registration")
        .client("CAP END")
        .update(|session| {
            assert_eq!(
                session.verify(std::time::Instant::now(), "alice", "pending-code"),
                Err(papo_core::RequestError::Unsupported(
                    "account verification before connection"
                ))
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
                .register(
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
        .event(|event| matches!(event, Event::AccountRegistered { account, .. } if account == "alice"))
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
                .register(std::time::Instant::now(), None, None, "password")
                .unwrap();
        })
        .server("CAP * ACK :draft/account-registration")
        .client("REGISTER * * password")
        .server(":ergo.test FAIL REGISTER WEAK_PASSWORD alice :Password is too weak")
        .event(|event| matches!(event, Event::RequestUntracked { .. }))
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.account_registration_error() == Some(AccountRegistrationError::WeakPassword))
        })
        .client("CAP END");
}
