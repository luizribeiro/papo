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
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

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
    wait: Option<Pin<Box<dyn Future<Output = ()>>>>,
    closed: bool,
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
            wait: None,
            closed: false,
        }
    }

    fn poll_below(
        &mut self,
        context: &mut Context<'_>,
        ceiling: u32,
    ) -> Poll<io::Result<()>> {
        if self.socket.ready_state() != web_sys::WebSocket::OPEN {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "browser WebSocket is not open",
            )));
        }
        if below(self.socket.buffered_amount(), ceiling) {
            self.wait = None;
            return Poll::Ready(Ok(()));
        }
        let wait = self.wait.get_or_insert_with(|| {
            Box::pin(crate::rt::sleep_until(
                crate::rt::now_mono() + Duration::from_millis(10),
            ))
        });
        if wait.as_mut().poll(context).is_ready() {
            self.wait = None;
            context.waker().wake_by_ref();
        }
        Poll::Pending
    }

    fn send(&self, bytes: &[u8]) -> io::Result<()> {
        match self.protocol {
            Protocol::Binary => self.socket.send_with_u8_array(bytes).map_err(send_error),
            Protocol::Text => {
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 text frame")
                })?;
                self.socket.send_with_str(text).map_err(send_error)
            }
        }
    }
}

fn below(amount: u32, ceiling: u32) -> bool {
    amount < ceiling
}

fn send_error(_error: wasm_bindgen::JsValue) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "browser rejected WebSocket send")
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
        if !self.closed {
            _ = self.socket.close_with_code(1000);
        }
    }
}

impl AsyncWrite for WebSocket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        std::task::ready!(self.poll_below(context, 65_536))?;
        match self.write.push(buffer)? {
            write::Write::Buffered(consumed) => Poll::Ready(Ok(consumed)),
            write::Write::Frame { consumed, bytes } => {
                self.send(&bytes)?;
                Poll::Ready(Ok(consumed))
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_below(context, 1)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.write.finish()?;
        if !self.closed {
            self.callbacks.take();
            self.socket
                .close_with_code(1000)
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "WebSocket close failed"))?;
            self.closed = true;
        }
        Poll::Ready(Ok(()))
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
    use tokio::io::AsyncWriteExt;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::WebSocket;

    #[wasm_bindgen_test]
    fn applies_the_literal_backpressure_threshold() {
        assert!(super::below(65_535, 65_536));
        assert!(!super::below(65_536, 65_536));
    }

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

    #[wasm_bindgen_test]
    async fn text_mode_rejects_invalid_utf8_and_shutdown_releases_callbacks() {
        let mut stream = WebSocket::connect_with(env!("PAPO_BROWSER_WS"), &[super::handshake::TEXT])
            .await
            .unwrap();
        assert_eq!(
            stream.write_all(&[0xff, b'\n']).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        let socket = stream.socket.clone();
        stream.shutdown().await.unwrap();
        assert!(socket.onmessage().is_none());
        assert!(socket.onerror().is_none());
        assert!(socket.onclose().is_none());
    }
}
