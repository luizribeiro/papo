use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A connected browser WebSocket byte stream.
#[derive(Debug)]
pub(crate) struct Stream {
    inner: Box<super::websocket_browser::WebSocket>,
}

impl Stream {
    pub(super) fn from_websocket(stream: super::websocket_browser::WebSocket) -> Self {
        Self {
            inner: Box::new(stream),
        }
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
