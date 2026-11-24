//! A refused channel key stays redacted when the reply is not valid UTF-8.

mod support;

use papo_core::{Config, Event, Secret};
use papo_proto::Line;
use support::Transcript;

const KEY: &str = "contraseña-4821";

fn refused(advertise_utf8_only: bool) -> Vec<String> {
    let mut reply = format!(":ergo.test 696 papo #staff k {KEY} :Clave inv").into_bytes();
    reply.extend_from_slice(b"\xe1lida");
    let mut shown = Vec::new();
    let mut transcript = Transcript::new(Config::default().nick("papo"))
        .connect_default()
        .server(":ergo.test CAP * LS :multi-prefix")
        .client("CAP REQ :multi-prefix")
        .server(":ergo.test CAP * ACK :multi-prefix")
        .client("CAP END")
        .server(":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo");
    if advertise_utf8_only {
        transcript =
            transcript.server(":ergo.test 005 papo UTF8ONLY :are supported by this server");
    }
    transcript
        .server(":ergo.test 422 papo :MOTD File is missing")
        .send(
            Line::new("MODE")
                .with_params(["#staff", "+k"])
                .param(Secret::new(KEY)),
        )
        .client(&format!("MODE #staff +k {KEY}"))
        .event_until(|event| matches!(event, Event::Connected(_)))
        .server_bytes(&reply)
        .event_until(|event| {
            shown.push(format!("{event:?}"));
            true
        })
        .inspect(|session| shown.push(format!("{session:?}")))
        .ignore_rest();
    shown
}

#[test]
fn a_refused_non_ascii_key_stays_redacted_in_a_reply_that_is_not_utf8() {
    let as_latin1: String = KEY.bytes().map(char::from).collect();
    for advertise_utf8_only in [true, false] {
        let shown = refused(advertise_utf8_only).join(" ");
        assert!(
            !shown.contains(KEY) && !shown.contains(&as_latin1),
            "UTF8ONLY advertised: {advertise_utf8_only}; the key is readable in: {shown}"
        );
    }
}
