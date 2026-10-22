//! Transcript tests for `draft/webpush` and `soju.im/webpush`.
#![cfg(feature = "unstable")]

mod support;

use papo_core::{Config, Event, Feature, Refusal, RequestError, Session};
use support::{Transcript, assert_session_debug_redacts, queue_debug_marker, unsupported_because};

const ENDPOINT: &str = "https://push.example.net/message/ErgoTest/alice";
const P256DH: &str = "AQIDBAUGBwgJCgsMDQ4P_w";
const AUTH: &str = "-vv8_f7_";

fn registered(cap: &str) -> Transcript {
    Transcript::new(
        Config::default()
            .enable(Feature::WebPush)
            .wanted_caps([cap, "labeled-response"]),
    )
    .connect_default()
    .server(&format!("CAP * LS :{cap} labeled-response"))
    .client(&format!("CAP REQ :{cap} labeled-response"))
    .server(&format!("CAP * ACK :{cap} labeled-response"))
    .client("CAP END")
    .welcome(":ergo.test 001 alice :Welcome to Ergo")
    .server(":ergo.test 422 alice :MOTD File is missing")
    .event(|event| matches!(event, Event::Connected(_)))
}

#[test]
fn registers_and_unregisters_webpush_endpoints_with_both_caps() {
    for cap in ["draft/webpush", "soju.im/webpush"] {
        registered(cap)
            .update(|session| {
                session
                    .webpush_register(
                        std::time::Instant::now(),
                        ENDPOINT,
                        P256DH,
                        &papo_core::Secret::new(AUTH),
                    )
                    .unwrap();
                queue_debug_marker(session);
            })
            .inspect(|session| assert_session_debug_redacts(session, AUTH))
            .client(&format!(
                "@label=0 WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={AUTH}"
            ))
            .client("PRIVMSG #papo :ordinary debug marker")
            .update(|session| {
                session
                    .webpush_unregister(std::time::Instant::now(), ENDPOINT)
                    .unwrap();
            })
            .client(&format!("@label=1 WEBPUSH UNREGISTER {ENDPOINT}"));
    }
}

#[test]
fn rejects_invalid_endpoints_and_keys_before_queueing() {
    let mut unsupported = Session::new(Config::default());
    assert_eq!(
        unsupported.webpush_unregister(std::time::Instant::now(), ENDPOINT),
        Err(unsupported_because(
            Feature::WebPush,
            papo_core::UnsupportedReason::DisabledByClient,
        ))
    );
    registered("draft/webpush")
        .update(|session| {
            for endpoint in ["http://push.example.net/alice", "https://"] {
                assert_eq!(
                    session.webpush_unregister(std::time::Instant::now(), endpoint),
                    Err(RequestError::InvalidWebPushEndpoint)
                );
            }
            for (p256dh, auth) in [
                ("not+url/base64", AUTH),
                (P256DH, "="),
                ("", AUTH),
                (P256DH, ""),
            ] {
                let auth = papo_core::Secret::new(auth);
                assert_eq!(
                    session.webpush_register(std::time::Instant::now(), ENDPOINT, p256dh, &auth),
                    Err(RequestError::InvalidWebPushKey)
                );
            }
            assert_eq!(
                session.webpush_register(
                    std::time::Instant::now(),
                    ENDPOINT,
                    P256DH,
                    &papo_core::Secret::from_bytes([0xff]),
                ),
                Err(RequestError::InvalidSecret)
            );
        })
        .no_client();
}

#[test]
fn types_every_webpush_failure() {
    for (code, expected) in [
        ("INVALID_PARAMS", Refusal::InvalidWebPushParameters),
        ("INTERNAL_ERROR", Refusal::InternalError),
        ("MAX_REGISTRATIONS", Refusal::WebPushRegistrationLimit),
    ] {
        registered("draft/webpush")
            .server(&format!(
                ":ergo.test FAIL WEBPUSH {code} :Web push request rejected"
            ))
            .event(|event| {
                matches!(event, Event::StandardReply(reply)
                if reply.refusal() == Some(expected))
            });
    }
    registered("draft/webpush")
        .server(":ergo.test FAIL REGISTER INVALID_PARAMS :Registration rejected")
        .event(|event| {
            matches!(event, Event::StandardReply(reply)
            if reply.refusal() == Some(Refusal::Other))
        });
}

#[test]
fn types_a_labelled_webpush_failure() {
    registered("draft/webpush")
        .update(|session| {
            session
                .webpush_unregister(std::time::Instant::now(), ENDPOINT)
                .unwrap();
        })
        .client(&format!("@label=0 WEBPUSH UNREGISTER {ENDPOINT}"))
        .server("@label=0 :ergo.test FAIL WEBPUSH MAX_REGISTRATIONS :Too many registrations")
        .event(|event| {
            matches!(event, Event::Response(response)
            if response.refusal() == Some(Refusal::WebPushRegistrationLimit))
        });
}
