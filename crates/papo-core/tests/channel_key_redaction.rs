//! Transcript coverage for channel-key redaction in raw events.

mod support;

use std::time::Instant;

use papo_core::{Config, Event, Secret};
use papo_proto::Line;
use support::Transcript;

fn raw_line_has_debug(expected: &str) -> impl FnOnce(&Event) -> bool + '_ {
    move |event| {
        let Event::Line(line) = event else {
            return false;
        };
        assert_eq!(format!("{:?}", line.clone().into_line()), expected);
        assert_eq!(
            format!("{event:?}").contains("hunter2"),
            expected.contains("hunter2")
        );
        true
    }
}

#[test]
fn redacts_only_channel_key_arguments_in_raw_events() {
    Transcript::new(Config::default().nick("papo"))
        .server(":ergo.test 005 papo CHANMODES=Ibe,k,fl,imnpst :are supported by this server")
        .server(":ergo.test BATCH +opaque vendor.example/archive")
        .server("@batch=opaque :alice!~alice@staff.example MODE #staff +k hunter2")
        .event(raw_line_has_debug(
            "@batch=opaque :alice!~alice@staff.example MODE #staff +k <redacted>",
        ))
        .server("@batch=opaque :alice!~alice@staff.example MODE #staff +lk 10 hunter2")
        .event(raw_line_has_debug(
            "@batch=opaque :alice!~alice@staff.example MODE #staff +lk 10 <redacted>",
        ))
        .server("@batch=opaque :alice!~alice@staff.example MODE papo +k hunter2")
        .event(raw_line_has_debug(
            "@batch=opaque :alice!~alice@staff.example MODE papo +k hunter2",
        ))
        .server("@batch=opaque :alice!~alice@staff.example MODE #staff -k *")
        .event(raw_line_has_debug(
            "@batch=opaque :alice!~alice@staff.example MODE #staff -k <redacted>",
        ))
        .server(":ergo.test BATCH -opaque")
        .server(":ergo.test 324 papo #staff +nk hunter2")
        .event(raw_line_has_debug(
            ":ergo.test 324 papo #staff +nk <redacted>",
        ))
        .no_event();
}

#[test]
fn rejected_channel_key_stays_redacted() {
    const KEY: &str = "hunter2,staff-only-6294";
    Transcript::new(Config::default().nick("papo").wanted_caps(["labeled-response"]))
        .connect_default()
        .server(":ergo.test CAP * LS :labeled-response")
        .client("CAP REQ :labeled-response")
        .server(":ergo.test CAP * ACK :labeled-response")
        .client("CAP END")
        .update(|session| {
            session.request(Instant::now(), Line::new("MODE").with_params(["#staff", "+k"]).param(Secret::new(KEY))).unwrap();
        })
        .server(":ergo.test 001 papo :Welcome to Ergo")
        .server(":ergo.test 422 papo :MOTD File is missing")
        .client(&format!("@label=0 MODE #staff +k {KEY}"))
        .server(&format!("@label=0 :ergo.test 696 papo #staff k {KEY} :Invalid key mode parameter. Syntax: <key>."))
        .inspect(|session| assert!(!format!("{session:?}").contains(KEY)))
        .event_until(|event| {
            let Event::Response(response) = event else { return false };
            let error = response.error().expect("expected server refusal");
            assert!(matches!(error, papo_core::ServerError::Numeric { ref params, .. } if params[3] == "<redacted>"));
            assert_eq!(response.lines[0].as_ref().param(3), Some("<redacted>"));
            true
        });
}
