//! Transcript coverage for channel-key redaction in raw events.

mod support;

use papo_core::{Config, Event};
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
