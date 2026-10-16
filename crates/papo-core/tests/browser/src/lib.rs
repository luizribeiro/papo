//! Minimal browser and size probe for `papo-core`.

use papo_core::{Config, Session};
use papo_proto::MessageRef;
use wasm_bindgen::prelude::wasm_bindgen;
use web_time::Instant;

/// Parses a representative message and advances a session through welcome.
///
/// # Panics
///
/// Panics if the embedded representative IRC line is invalid.
#[must_use]
#[wasm_bindgen]
pub fn core_probe() -> usize {
    let message = MessageRef::parse(
        "@time=2026-10-16T12:34:56.789Z :emily!~em@localhost PRIVMSG #papo :hello",
    )
    .expect("the probe line is valid IRC");
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-browser"));
    session.handle_connected(now);
    session.handle_line(now, b":ergo.test 001 papo-browser :Welcome to Ergo");
    message.raw_command().len() + session.nick().len()
}
