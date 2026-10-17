use std::{cell::RefCell, io, rc::Rc};

use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{CloseEvent, ErrorEvent, MessageEvent};

use super::{handshake::Protocol, read::ReadState};

pub(super) struct Callbacks {
    socket: web_sys::WebSocket,
    message: Closure<dyn FnMut(MessageEvent)>,
    error: Closure<dyn FnMut(ErrorEvent)>,
    close: Closure<dyn FnMut(CloseEvent)>,
}

impl Callbacks {
    pub(super) fn install(
        socket: &web_sys::WebSocket,
        protocol: Protocol,
        state: Rc<RefCell<ReadState>>,
    ) -> Self {
        let messages = Rc::clone(&state);
        let message = Closure::wrap(Box::new(move |event: MessageEvent| {
            match decode(event.data(), protocol) {
                Ok(bytes) => messages.borrow_mut().push(bytes),
                Err(error) => messages.borrow_mut().fail(error),
            }
        }) as Box<dyn FnMut(_)>);
        let errors = Rc::clone(&state);
        let error = Closure::wrap(Box::new(move |_event: ErrorEvent| {
            errors.borrow_mut().fail(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "browser WebSocket reported an error",
            ));
        }) as Box<dyn FnMut(_)>);
        let closing = socket.clone();
        let close = Closure::wrap(Box::new(move |event: CloseEvent| {
            state.borrow_mut().close(event.code(), &event.reason());
            closing.set_onmessage(None);
            closing.set_onerror(None);
            closing.set_onclose(None);
        }) as Box<dyn FnMut(_)>);
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        socket.set_onerror(Some(error.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        Self {
            socket: socket.clone(),
            message,
            error,
            close,
        }
    }
}

impl Drop for Callbacks {
    fn drop(&mut self) {
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        _ = (&self.message, &self.error, &self.close);
    }
}

fn decode(value: JsValue, protocol: Protocol) -> io::Result<Vec<u8>> {
    match protocol {
        Protocol::Binary if value.is_instance_of::<ArrayBuffer>() => {
            Ok(Uint8Array::new(&value).to_vec())
        }
        Protocol::Text => value
            .as_string()
            .map(String::into_bytes)
            .ok_or_else(wrong_type),
        Protocol::Binary => Err(wrong_type()),
    }
}

fn wrong_type() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "WebSocket message type does not match its IRCv3 subprotocol",
    )
}

#[cfg(test)]
mod tests {
    use js_sys::Uint8Array;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{Protocol, decode};

    #[wasm_bindgen_test]
    fn decodes_only_the_negotiated_message_type() {
        let text = JsValue::from_str(":ergo.test CAP papo LS :message-tags");
        assert_eq!(
            decode(text.clone(), Protocol::Text).unwrap(),
            b":ergo.test CAP papo LS :message-tags"
        );
        assert!(decode(text, Protocol::Binary).is_err());

        let array = Uint8Array::from(b":ergo.test 001 papo :Welcome".as_slice()).buffer();
        assert_eq!(
            decode(array.clone().into(), Protocol::Binary).unwrap(),
            b":ergo.test 001 papo :Welcome"
        );
        assert!(decode(array.into(), Protocol::Text).is_err());
    }
}
