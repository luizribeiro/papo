use std::{cell::RefCell, rc::Rc};

use js_sys::Array;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{BinaryType, CloseEvent, ErrorEvent, Event};

use super::pure::{BINARY, Protocol, TEXT, select_protocol};

struct Handshake {
    socket: web_sys::WebSocket,
    open: Closure<dyn FnMut(Event)>,
    error: Closure<dyn FnMut(ErrorEvent)>,
    close: Closure<dyn FnMut(CloseEvent)>,
}

impl Drop for Handshake {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        _ = (&self.open, &self.error, &self.close);
    }
}

pub(super) async fn open(url: &str) -> Result<(web_sys::WebSocket, Protocol), String> {
    open_with(url, &[BINARY, TEXT]).await
}

pub(super) async fn open_with(
    url: &str,
    offered: &[&str],
) -> Result<(web_sys::WebSocket, Protocol), String> {
    let protocols = Array::new();
    for protocol in offered {
        protocols.push(&(*protocol).into());
    }
    let socket = web_sys::WebSocket::new_with_str_sequence(url, &protocols)
        .map_err(|_| "browser rejected WebSocket construction".to_owned())?;
    socket.set_binary_type(BinaryType::Arraybuffer);

    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let opened = Rc::clone(&sender);
    let open = Closure::wrap(Box::new(move |_event: Event| {
        if let Some(sender) = opened.borrow_mut().take() {
            _ = sender.send(Ok(()));
        }
    }) as Box<dyn FnMut(_)>);
    let failed = Rc::clone(&sender);
    let error = Closure::wrap(Box::new(move |_event: ErrorEvent| {
        if let Some(sender) = failed.borrow_mut().take() {
            _ = sender.send(Err("WebSocket opening handshake failed".to_owned()));
        }
    }) as Box<dyn FnMut(_)>);
    let closed = Rc::clone(&sender);
    let close = Closure::wrap(Box::new(move |event: CloseEvent| {
        if let Some(sender) = closed.borrow_mut().take() {
            _ = sender.send(Err(format!(
                "WebSocket closed with code {} during opening handshake",
                event.code()
            )));
        }
    }) as Box<dyn FnMut(_)>);
    socket.set_onopen(Some(open.as_ref().unchecked_ref()));
    socket.set_onerror(Some(error.as_ref().unchecked_ref()));
    socket.set_onclose(Some(close.as_ref().unchecked_ref()));
    let handshake = Handshake {
        socket,
        open,
        error,
        close,
    };
    receiver
        .await
        .map_err(|_| "WebSocket opening callbacks were released".to_owned())??;
    let protocol = select_protocol(&handshake.socket.protocol()).map_err(str::to_owned)?;
    let socket = handshake.socket.clone();
    drop(handshake);
    Ok((socket, protocol))
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{Protocol, open, open_with};

    #[wasm_bindgen_test]
    async fn offers_binary_before_text_and_requests_arraybuffers() {
        let (socket, protocol) = open(env!("PAPO_BROWSER_WS")).await.unwrap();
        assert_eq!(protocol, Protocol::Binary);
        assert_eq!(socket.binary_type(), web_sys::BinaryType::Arraybuffer);
        assert!(socket.onopen().is_none());
        assert!(socket.onerror().is_none());
        assert!(socket.onclose().is_none());
        socket.close().unwrap();
    }

    #[wasm_bindgen_test]
    async fn reports_handshake_failure_and_can_offer_text_only() {
        assert!(open("ws://127.0.0.1:1/").await.is_err());
        let (socket, protocol) = open_with(env!("PAPO_BROWSER_WS"), &[super::TEXT])
            .await
            .unwrap();
        assert_eq!(protocol, Protocol::Text);
        socket.close().unwrap();
    }
}
