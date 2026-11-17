//! A bouncer that re-advertises SASL after registration never receives the login again.

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo::{Reconnect, Sasl};
use papo_testing::{Wire, wire};

const PASSWORD: &str = "bouncer-login-password-6083";

async fn expect_sent(wire: &mut Wire, expected: &str) {
    assert_eq!(wire.next_sent().await, expected);
}

async fn log_in_to_the_bouncer(wire: &mut Wire, success: &str) {
    expect_sent(wire, "CAP LS 302").await;
    expect_sent(wire, "NICK papo-wire").await;
    expect_sent(wire, "USER papo-wire 0 * papo-wire").await;
    wire.server(":bouncer.papo.test CAP * LS :sasl=PLAIN").await;
    expect_sent(wire, "CAP REQ :sasl").await;
    wire.server(":bouncer.papo.test CAP * ACK :sasl").await;
    expect_sent(wire, "AUTHENTICATE PLAIN").await;
    wire.server("AUTHENTICATE +").await;
    expect_sent(
        wire,
        &format!(
            "AUTHENTICATE {}",
            STANDARD.encode(format!("\0alice\0{PASSWORD}"))
        ),
    )
    .await;
    wire.server(success).await;
    expect_sent(wire, "CAP END").await;
}

async fn welcome_before_the_upstream_registers(wire: &mut Wire) {
    wire.server(":bouncer.papo.test CAP * DEL sasl").await;
    wire.server(":bouncer.papo.test 001 papo-wire :Welcome to soju, papo-wire")
        .await;
    wire.server(":bouncer.papo.test 005 papo-wire CASEMAPPING=ascii CHANTYPES=# :are supported")
        .await;
    wire.server(":bouncer.papo.test 422 papo-wire :No MOTD")
        .await;
}

async fn assert_still_connected(events: &mut papo::Events) {
    while let Ok(Some((event, _))) =
        tokio::time::timeout(Duration::from_millis(50), events.next()).await
    {
        assert!(
            !matches!(event, papo::Event::Disconnected(_)),
            "client disconnected: {event:?}"
        );
    }
}

#[tokio::test]
async fn bouncer_login_never_reaches_a_late_sasl_offer() {
    for success in [
        ":bouncer.papo.test 903 papo-wire :SASL authentication successful",
        ":bouncer.papo.test 907 papo-wire :You have already authenticated using SASL",
    ] {
        let (_client, mut events, mut wire) = wire(
            papo::Client::builder("irc://bouncer.papo.test", "papo-wire")
                .rate_limit(papo::RateLimit::unlimited())
                .reconnect(Reconnect::never())
                .sasl(Sasl::plain("alice", papo::Secret::new(PASSWORD)))
                .allow_insecure_auth(true),
        );
        log_in_to_the_bouncer(&mut wire, success).await;
        welcome_before_the_upstream_registers(&mut wire).await;

        wire.server(":bouncer.papo.test CAP papo-wire NEW sasl=PLAIN,ANONYMOUS")
            .await;
        expect_sent(&mut wire, "CAP REQ :sasl").await;
        wire.server(":bouncer.papo.test CAP papo-wire ACK sasl")
            .await;
        wire.server("PING :bouncer.papo.test").await;
        expect_sent(&mut wire, "PONG bouncer.papo.test").await;
        assert_still_connected(&mut events).await;
    }
}
