//! Browser and size probes for the protocol core and high-level driver.

use papo_core::{Config, Session};
use papo_proto::LineRef;
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
    let message =
        LineRef::parse("@time=2026-10-16T12:34:56.789Z :emily!~em@localhost PRIVMSG #papo :hello")
            .expect("the probe line is valid IRC");
    let now = Instant::now();
    let mut session = Session::new(Config::default().nick("papo-browser"));
    session.handle_connected(now);
    session.handle_line(now, b":ergo.test 001 papo-browser :Welcome to Ergo");
    message.raw_command().len() + session.nick().len()
}

#[cfg(feature = "driver")]
mod driver {
    use papo::{Client, Endpoint};

    use super::*;

    /// Connects and cleanly closes the high-level browser driver.
    ///
    /// # Errors
    ///
    /// Returns the public connection or shutdown diagnostic as a JavaScript string.
    #[wasm_bindgen]
    pub async fn driver_probe(url: String) -> Result<(), wasm_bindgen::JsValue> {
        let (client, _) = Client::builder(Endpoint::websocket(url).map_err(js_error)?, "papo-browser-probe")
            .connect()
            .await
            .map_err(js_error)?;
        client
            .quit("browser probe complete")
            .await
            .map_err(js_error)
    }

    fn js_error(error: impl std::fmt::Display) -> wasm_bindgen::JsValue {
        wasm_bindgen::JsValue::from_str(&error.to_string())
    }
}
