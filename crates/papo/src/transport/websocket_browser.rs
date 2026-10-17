#[path = "websocket_browser/callbacks.rs"]
mod callbacks;
#[path = "websocket_browser/handshake.rs"]
mod handshake;
#[path = "websocket_browser/read.rs"]
mod read;
#[path = "websocket_browser/write.rs"]
mod write;

use std::{
    cell::RefCell,
    fmt, io,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, ReadBuf};

use callbacks::Callbacks;
use handshake::Protocol;
use read::ReadState;
use write::WriteState;

pub(super) struct WebSocket {
    socket: web_sys::WebSocket,
    protocol: Protocol,
    read: Rc<RefCell<ReadState>>,
    write: WriteState,
    callbacks: Option<Callbacks>,
}

impl WebSocket {
    pub(super) async fn connect(url: &str) -> Result<Self, String> {
        let (socket, protocol) = handshake::open(url).await?;
        Ok(Self::from_open(socket, protocol))
    }

    #[cfg(test)]
    async fn connect_with(url: &str, offered: &[&str]) -> Result<Self, String> {
        let (socket, protocol) = handshake::open_with(url, offered).await?;
        Ok(Self::from_open(socket, protocol))
    }

    fn from_open(socket: web_sys::WebSocket, protocol: Protocol) -> Self {
        let read = Rc::new(RefCell::new(ReadState::default()));
        let callbacks = Callbacks::install(&socket, protocol, Rc::clone(&read));
        Self {
            socket,
            protocol,
            read,
            write: WriteState::default(),
            callbacks: Some(callbacks),
        }
    }
}

impl fmt::Debug for WebSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocket")
            .field("protocol", &self.protocol)
            .field("buffered_write_bytes", &self.write.len())
            .finish_non_exhaustive()
    }
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        self.callbacks.take();
        _ = self.socket.close_with_code(1000);
    }
}

impl AsyncRead for WebSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.read.borrow_mut().poll_read(context, buffer)
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::WebSocket;

    #[wasm_bindgen_test]
    async fn dropping_a_stream_releases_every_callback() {
        let stream = WebSocket::connect(env!("PAPO_BROWSER_WS")).await.unwrap();
        let socket = stream.socket.clone();
        assert!(socket.onmessage().is_some());
        assert!(socket.onerror().is_some());
        assert!(socket.onclose().is_some());
        drop(stream);
        assert!(socket.onmessage().is_none());
        assert!(socket.onerror().is_none());
        assert!(socket.onclose().is_none());
    }
}
