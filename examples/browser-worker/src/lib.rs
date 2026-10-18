//! `SharedWorker` entry point for forwarding IRC events to a browser page.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use papo::{Client, Endpoint, Event};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::wasm_bindgen;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_sys::MessagePort;

/// Starts one IRC client and posts each event to the owning page.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen]
pub fn run(port: MessagePort, url: String, nick: String, channel: String) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = forward_events(&port, &url, &nick, &channel).await {
            post(&port, &format!("error: {error}"));
        }
    });
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn forward_events(
    port: &MessagePort,
    url: &str,
    nick: &str,
    channel: &str,
) -> Result<(), papo::Error> {
    let (client, mut events) = Client::builder(Endpoint::websocket(url))
        .nick(nick)
        .autojoin([channel])
        .connect()
        .await?;
    while let Some(event) = events.next().await {
        post(port, &format!("{event:?}"));
        if matches!(event, Event::Disconnected(ref disconnected) if !disconnected.is_retrying()) {
            break;
        }
    }
    drop(client);
    Ok(())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn post(port: &MessagePort, message: &str) {
    _ = port.post_message(&message.into());
}
