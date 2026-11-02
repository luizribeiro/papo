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

fn registered_without_labels(cap: &str) -> Transcript {
    Transcript::new(
        Config::default()
            .enable(Feature::WebPush)
            .wanted_caps([cap]),
    )
    .connect_default()
    .server(&format!("CAP * LS :{cap}"))
    .client(&format!("CAP REQ :{cap}"))
    .server(&format!("CAP * ACK :{cap}"))
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
            .server(&format!(
                "@label=0 :ergo.test WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={AUTH}"
            ))
            .event(|event| matches!(event, Event::Response(_)))
            .client("PRIVMSG #papo :ordinary debug marker")
            .update(|session| {
                session
                    .webpush_unregister(std::time::Instant::now(), ENDPOINT)
                    .unwrap();
            })
            .client(&format!("@label=1 WEBPUSH UNREGISTER {ENDPOINT}"))
            .server(&format!(
                "@label=1 :ergo.test WEBPUSH UNREGISTER {ENDPOINT}"
            ))
            .event(|event| matches!(event, Event::Response(_)));
    }
}

#[test]
fn preserves_labelled_registration_warnings_in_ergo_order() {
    registered("draft/webpush")
        .update(|session| {
            session
                .webpush_register(
                    std::time::Instant::now(),
                    ENDPOINT,
                    P256DH,
                    &papo_core::Secret::new(AUTH),
                )
                .unwrap();
        })
        .client(&format!(
            "@label=0 WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={AUTH}"
        ))
        .server("@label=0 :ergo.test BATCH +webpush labeled-response")
        .no_event()
        .server(&format!(
            "@batch=webpush :ergo.test WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={AUTH}"
        ))
        .no_event()
        .server("@batch=webpush :ergo.test WARN WEBPUSH PERSISTENCE_REQUIRED :You have enabled push notifications, but you will not receive them unless you become always-on")
        .no_event()
        .server(":ergo.test BATCH -webpush")
        .event(|event| {
            matches!(event, Event::Response(response)
                if response.lines.len() == 2
                    && response.lines[0].as_ref().command_is("WEBPUSH")
                    && response.lines[1].as_ref().param(1) == Some("PERSISTENCE_REQUIRED"))
        });
}

#[test]
fn completes_unlabelled_webpush_replies_and_redacts_echoed_keys() {
    const SECRET_AUTH: &str = "dW5pcXVlLXdlYnB1c2gtYXV0aC00ODYy";

    registered_without_labels("draft/webpush")
        .update(|session| {
            session
                .webpush_register(
                    std::time::Instant::now(),
                    ENDPOINT,
                    P256DH,
                    &papo_core::Secret::new(SECRET_AUTH),
                )
                .unwrap();
        })
        .client(&format!(
            "WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={SECRET_AUTH}"
        ))
        .client("PING papo-webpush-0")
        .server(&format!(
            ":ergo.test WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={SECRET_AUTH}"
        ))
        .no_event()
        .server(":ergo.test WARN WEBPUSH PERSISTENCE_REQUIRED :You have enabled push notifications, but you will not receive them unless you become always-on")
        .no_event()
        .server(":ergo.test PONG ergo.test :papo-webpush-0")
        .inspect(|session| assert!(!format!("{session:?}").contains(SECRET_AUTH)))
        .event(|event| {
            let Event::Response(response) = event else {
                return false;
            };
            let debug = format!("{response:?}");
            !debug.contains(SECRET_AUTH)
                && response.lines.len() == 3
                && response.lines[0].get(2) == Some("<redacted>")
                && response.lines[1].as_ref().param(1) == Some("PERSISTENCE_REQUIRED")
        })
        .update(|session| {
            session
                .webpush_register(
                    std::time::Instant::now(),
                    ENDPOINT,
                    P256DH,
                    &papo_core::Secret::new(SECRET_AUTH),
                )
                .unwrap();
        })
        .client(&format!(
            "WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={SECRET_AUTH}"
        ))
        .client("PING papo-webpush-1")
        .server(&format!(
            ":ergo.test WEBPUSH REGISTER {ENDPOINT} p256dh={P256DH};auth={SECRET_AUTH}"
        ))
        .no_event()
        .server(":ergo.test PONG ergo.test :papo-webpush-1")
        .event(|event| {
            matches!(event, Event::Response(response)
                if response.lines.len() == 2
                    && response.lines.iter().all(|line| {
                        line.as_ref().param(1) != Some("PERSISTENCE_REQUIRED")
                    }))
        })
        .update(|session| {
            session
                .webpush_unregister(std::time::Instant::now(), ENDPOINT)
                .unwrap();
        })
        .client(&format!("WEBPUSH UNREGISTER {ENDPOINT}"))
        .server(":ergo.test FAIL WEBPUSH INVALID_PARAMS UNREGISTER https://push.example.net/message/ErgoTest/alice :Invalid web push URL")
        .event(|event| {
            matches!(event, Event::Response(response)
                if response.refusal() == Some(Refusal::InvalidWebPushParameters))
        })
        .update(|session| {
            session
                .webpush_unregister(std::time::Instant::now(), ENDPOINT)
                .unwrap();
        })
        .client(&format!("WEBPUSH UNREGISTER {ENDPOINT}"))
        .server(&format!(":ergo.test WEBPUSH UNREGISTER {ENDPOINT}"))
        .event(|event| matches!(event, Event::Response(_)))
        .server(":ergo.test EXTENSION REGISTER account.example ordinary-diagnostic")
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.as_ref().param(2) == Some("ordinary-diagnostic"))
        })
        .server(&format!(
            ":ergo.test WEBPUSH UNREGISTER {ENDPOINT} ordinary-diagnostic"
        ))
        .event(|event| {
            matches!(event, Event::Line(line)
                if line.as_ref().param(2) == Some("ordinary-diagnostic"))
        });
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
        })
        .no_client();
}

#[test]
fn types_every_webpush_failure() {
    for (code, parameters, expected) in [
        ("INVALID_PARAMS", "", Refusal::InvalidWebPushParameters),
        ("INTERNAL_ERROR", "", Refusal::InternalError),
        ("MAX_REGISTRATIONS", "", Refusal::WebPushRegistrationLimit),
        (
            "FORBIDDEN",
            " UNREGISTER https://push.example.net/message/ErgoTest/alice",
            Refusal::WebPushForbidden,
        ),
    ] {
        registered("draft/webpush")
            .server(&format!(
                ":ergo.test FAIL WEBPUSH {code}{parameters} :Web push request rejected"
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
