//! Transcript tests for connection progress.

mod support;

use papo_core::{CloseKind, Config, ConnectPhase, Event, Sasl};
use support::Transcript;

#[test]
fn reports_capabilities_then_waiting_for_welcome() {
    Transcript::new(Config::default())
        .connect()
        .phase(ConnectPhase::Capabilities)
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server(":ergo.test CAP * LS :batch server-time")
        .client("CAP REQ :server-time batch")
        .server(":ergo.test CAP * ACK :server-time batch")
        .phase(ConnectPhase::WaitingForWelcome)
        .client("CAP END")
        .welcome(":ergo.test 001 papo :Welcome to the Ergo test network")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .event(|event| matches!(event, Event::Connected(_)));
}

#[test]
fn reports_authentication_and_ends_a_failed_phase() {
    Transcript::new(Config::default().sasl(Sasl::plain("alice", "correct horse battery staple")))
        .connect()
        .phase(ConnectPhase::Capabilities)
        .client("CAP LS 302")
        .client("NICK papo")
        .client("USER papo 0 * papo")
        .server(":ergo.test CAP * LS :sasl=PLAIN")
        .client("CAP REQ :sasl")
        .server(":ergo.test CAP * ACK :sasl")
        .phase(ConnectPhase::Authenticating)
        .client("AUTHENTICATE PLAIN")
        .server("AUTHENTICATE +")
        .client("AUTHENTICATE AGFsaWNlAGNvcnJlY3QgaG9yc2UgYmF0dGVyeSBzdGFwbGU=")
        .server(":ergo.test 904 papo :SASL authentication failed")
        .event(|event| {
            matches!(
                event,
                Event::Close {
                    kind: CloseKind::SaslFailed,
                    ..
                }
            )
        })
        .no_client();
}
