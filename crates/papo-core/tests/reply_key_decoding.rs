//! A refused channel key stays redacted when the reply is not valid UTF-8.

mod support;

use papo_core::{Config, Event, Secret};
use papo_proto::Line;
use support::Transcript;

const KEY: &str = "contraseña-4821";

fn refused(nick: &str, advertise_utf8_only: bool, key: &str, reply: &[u8]) -> Vec<String> {
    let malformed = advertise_utf8_only && std::str::from_utf8(reply).is_err();
    let mut shown = Vec::new();
    let mut transcript = Transcript::new(Config::default().nick(nick))
        .connect()
        .client("CAP LS 302")
        .client(&format!("NICK {nick}"))
        .client("USER papo 0 * papo")
        .server(":ergo.test CAP * LS :multi-prefix")
        .client("CAP REQ :multi-prefix")
        .server(":ergo.test CAP * ACK :multi-prefix")
        .client("CAP END")
        .server(&format!(
            ":ergo.test 001 {nick} :Welcome to the ErgoTest IRC Network {nick}"
        ));
    if advertise_utf8_only {
        transcript =
            transcript.server(":ergo.test 005 papo UTF8ONLY :are supported by this server");
    }
    transcript
        .server(":ergo.test 422 papo :MOTD File is missing")
        .send(
            Line::new("MODE")
                .with_params(["#staff", "+k"])
                .param(Secret::new(key)),
        )
        .client(&format!("MODE #staff +k {key}"))
        .event_until(|event| matches!(event, Event::Connected(_)))
        .server_bytes(reply)
        .event_until(|event| {
            let line = match &event {
                Event::Malformed { line, .. } => {
                    assert!(malformed);
                    String::from_utf8_lossy(line).into_owned()
                }
                Event::Line(line) => {
                    assert!(!malformed);
                    assert_eq!(line.get(0), Some(nick));
                    assert_eq!(line.get(3), Some("<redacted>"));
                    line.to_string()
                }
                _ => panic!("unexpected reply {event:?}"),
            };
            assert!(line.contains(" k <redacted> :Clave inv"), "{line}");
            shown.push(format!("{event:?} {line}"));
            true
        })
        .inspect(|session| shown.push(format!("{session:?}")))
        .ignore_rest();
    shown
}

#[test]
fn a_refused_non_ascii_key_stays_redacted_in_a_reply_that_is_not_utf8() {
    let mut reply = format!(":ergo.test 696 papo #staff k {KEY} :Clave inv").into_bytes();
    reply.extend_from_slice(b"\xe1lida");
    let as_latin1: String = KEY.bytes().map(char::from).collect();
    for advertise_utf8_only in [true, false] {
        let shown = refused("papo", advertise_utf8_only, KEY, &reply).join(" ");
        assert!(
            !shown.contains(KEY) && !shown.contains(&as_latin1),
            "UTF8ONLY advertised: {advertise_utf8_only}; the key is readable in: {shown}"
        );
    }
}

#[test]
fn keys_reencoded_by_legacy_servers_stay_redacted() {
    for (key, reply) in [
        (
            KEY,
            &b":ergo.test 696 papo #staff k contrase\xf1a-4821 :Clave inv\xe1lida"[..],
        ),
        (
            KEY,
            &b":ergo.test 696 papo #staff k contrase\xf1a-4821 :Clave invalida"[..],
        ),
        (
            "presupuesto€-4821",
            &b":ergo.test 696 papo #staff k presupuesto\x80-4821 :Clave inv\xe1lida"[..],
        ),
    ] {
        for utf8_only in [true, false] {
            let shown = refused("papo", utf8_only, key, reply).join(" ");
            assert!(!shown.contains(key) && !shown.contains("-4821"), "{shown}");
        }
    }
}

#[test]
fn valid_utf8_replies_retain_the_unmasked_text() {
    let reply = format!(":ergo.test 696 papo #staff k {KEY} :Clave inválida: {KEY}.");
    let shown = refused("papo", true, KEY, reply.as_bytes()).join(" ");
    assert!(shown.contains("Clave inválida: <redacted>."), "{shown}");
}

#[test]
fn a_legacy_reply_keeps_its_own_addressee_in_the_negotiated_encoding() {
    refused(
        "iñaki",
        false,
        "iñaki",
        b":ergo.test 696 i\xf1aki #staff k i\xf1aki :Clave inv\xe1lida",
    );
}
