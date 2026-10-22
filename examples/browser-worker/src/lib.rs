//! `SharedWorker` entry point for forwarding IRC events to a browser page.
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use papo::Client;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::MessagePort;

/// Starts one IRC client and posts each event to the owning page.
#[wasm_bindgen]
pub fn run(port: MessagePort, url: String, nick: String) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = forward_events(&port, url, nick).await {
            post(&port, &format!("error: {error}"));
        }
    });
}

async fn forward_events(port: &MessagePort, url: String, nick: String) -> Result<(), papo::Error> {
    let (_client, mut events) = Client::builder(url).nick(nick).connect().await?;
    while let Some(event) = events.next().await {
        post(port, &format!("{event:?}"));
    }
    Ok(())
}

fn post(port: &MessagePort, message: &str) {
    _ = port.post_message(&message.into());
}
