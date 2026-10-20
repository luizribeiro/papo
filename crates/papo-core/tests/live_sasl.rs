//! End-to-end SASL authentication against the pinned Ergo test server.
#![cfg(any(
    not(target_os = "wasi"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

mod support;

use std::{net::TcpStream, time::Instant};

use papo_core::{Config, Entropy, Event, RateLimit, Sasl, Session};
use papo_test_server::{ServerInfo, from_env, register_account, unique};
use support::{drive_until, within};

struct FixedEntropy;

impl Entropy for FixedEntropy {
    fn fill(&mut self, buf: &mut [u8]) {
        buf.fill(0x5a);
    }
}

#[test]
fn authenticates_with_plain_and_scram_against_live_ergo() {
    let server = from_env();
    let account = unique("papo-sasl");
    let password = "q7Hx-2mZp_9vLcT4wRfB";
    register_account(&server, &account, password).unwrap();

    for sasl in [
        Sasl::plain(&account, password),
        Sasl::scram_sha256(&account, password),
    ] {
        let session = authenticate(&server, &account, sasl)
            .unwrap_or_else(|reason| panic!("valid credentials closed: {reason}"));
        assert!(session.is_registered());
        assert_eq!(session.account(), Some(account.as_str()));
    }

    let reason = authenticate(
        &server,
        &account,
        Sasl::plain(&account, "wrong-password-4917"),
    )
    .expect_err("required SASL accepted a wrong password");
    assert!(reason.contains("SASL failed"));
}

fn authenticate(server: &ServerInfo, account: &str, sasl: Sasl) -> Result<Session, String> {
    let mut stream = TcpStream::connect(server.plaintext_addr()).unwrap();
    let config = Config::default()
        .nick(unique("papo-auth"))
        .username("papo")
        .realname("papo live SASL test")
        .sasl(sasl)
        .rate_limit(RateLimit::unlimited());
    let mut session = Session::with_entropy(config, Box::new(FixedEntropy));
    session.handle_connected(Instant::now());

    let event = within(&mut stream, |stream| {
        drive_until(&mut session, stream, 128, |event| {
            matches!(event, Event::Connected(_) | Event::Close { .. })
        })
    })
    .unwrap_or_else(|| panic!("Ergo did not finish SASL for {account}"));
    match event {
        Event::Connected(_) => Ok(session),
        Event::Close { reason, .. } => Err(reason),
        _ => unreachable!("drive predicate only accepts terminal events"),
    }
}
