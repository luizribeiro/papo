use papo_core::{Config, Event, Session};
use papo_proto::MessageRef;
use wasm_bindgen_test::wasm_bindgen_test;
use web_time::Instant;
#[path = "../../../../../papo/src/rt.rs"]
mod papo_rt;

#[wasm_bindgen_test]
fn parses_an_ergo_message() {
    let message = MessageRef::parse(
        "@time=2026-10-16T12:34:56.789Z :emily!~em@localhost PRIVMSG #papo :hello browser",
    )
    .unwrap();
    assert_eq!(message.raw_command(), "PRIVMSG");
    assert_eq!(message.raw_source(), Some("emily!~em@localhost"));
    assert_eq!(message.params().last(), Some("hello browser"));
}

#[wasm_bindgen_test]
fn drives_a_registration_transcript() {
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-browser"));
    session.handle_connected(now);
    assert_eq!(take_line(&mut session), "CAP LS 302");
    assert_eq!(take_line(&mut session), "NICK papo-browser");
    assert_eq!(take_line(&mut session), "USER papo 0 * papo");

    session.handle_line(now, b":ergo.test 001 papo-browser :Welcome to Ergo");
    assert_eq!(session.nick(), "papo-browser");
    assert!(matches!(session.poll_event(), Some(Event::Message(message)) if message.as_ref().command_is("001")));
}

#[wasm_bindgen_test]
fn reads_a_monotonic_instant() {
    let earlier = Instant::now();
    let later = Instant::now();
    assert!(later >= earlier);
}

#[wasm_bindgen_test]
fn receives_the_local_ergo_websocket_url() {
    _ = papo_rt::now_wall();
    let url = env!("PAPO_BROWSER_WS");
    assert!(url.starts_with("ws://127.0.0.1:"));
    assert!(url.ends_with('/'));
}

fn take_line(session: &mut Session) -> String {
    String::from_utf8(session.poll_transmit().expect("client line is available"))
        .unwrap()
        .trim_end()
        .to_owned()
}
