use std::{
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
#[cfg(tls)]
use tokio_rustls::client::TlsStream;

#[cfg(all(test, feature = "websocket"))]
#[path = "stream_sockets/broken.rs"]
mod broken;

trait Io: AsyncRead + AsyncWrite + fmt::Debug + Send + Unpin {}

impl<T> Io for T where T: AsyncRead + AsyncWrite + fmt::Debug + Send + Unpin {}

/// A connected IRC byte stream.
#[derive(Debug)]
pub(crate) struct Stream {
    inner: Box<dyn Io>,
    #[allow(dead_code)]
    secure: bool,
    #[allow(dead_code)]
    subprotocol: Option<&'static str>,
}

impl Stream {
    fn new(inner: impl Io + 'static, secure: bool, subprotocol: Option<&'static str>) -> Self {
        Self {
            inner: Box::new(inner),
            secure,
            subprotocol,
        }
    }

    pub(super) fn from_tcp(stream: TcpStream) -> Self {
        Self::new(stream, false, None)
    }

    #[cfg(tls)]
    pub(super) fn from_tls(stream: TlsStream<TcpStream>) -> Self {
        Self::new(stream, true, None)
    }

    #[cfg(feature = "websocket")]
    pub(super) fn from_websocket(stream: super::websocket::WebSocket) -> Self {
        let secure = stream.is_secure();
        let subprotocol = stream.subprotocol();
        Self::new(stream, secure, subprotocol)
    }

    #[cfg(all(test, feature = "websocket"))]
    pub(super) fn broken_peer() -> Self {
        Self::new(broken::BrokenPeer, false, None)
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn is_secure(&self) -> bool {
        self.secure
    }

    #[must_use]
    #[cfg(all(test, feature = "websocket"))]
    pub(crate) const fn websocket_subprotocol(&self) -> Option<&str> {
        self.subprotocol
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut()).poll_read(context, buffer)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(self.inner.as_mut()).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut()).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.inner.as_mut()).poll_shutdown(context)
    }
}
