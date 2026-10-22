use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) trait FutureStream:
    futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin
{
}

impl<T> FutureStream for T where T: futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin {}

pub(crate) struct FuturesIo {
    inner: Box<dyn FutureStream>,
}

impl FuturesIo {
    pub(crate) fn new(inner: Box<dyn FutureStream>) -> Self {
        Self { inner }
    }
}

impl AsyncRead for FuturesIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let available = buf.initialize_unfilled();
        match Pin::new(&mut *self.inner).poll_read(cx, available) {
            Poll::Ready(Ok(read)) => {
                buf.advance(read);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for FuturesIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.inner).poll_close(cx)
    }
}
