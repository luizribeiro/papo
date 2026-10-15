//! Transcript tests for `draft/account-registration`.

mod support;

use papo_core::{Config, Event};
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
