use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Debug)]
pub(super) struct BrokenPeer;

impl AsyncRead for BrokenPeer {
    fn poll_read(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(error()))
    }
}

impl AsyncWrite for BrokenPeer {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Poll::Ready(Err(error()))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(error()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(error()))
    }
}

fn error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "broken WebSocket peer")
}
