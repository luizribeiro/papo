//! Network connections to IRC servers.

#[cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]
use std::net::SocketAddr;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

/// The security used for a connection.
#[derive(Clone, Debug)]
pub enum Security {
    /// An unencrypted TCP connection.
    Plaintext,
}

/// A connected IRC byte stream.
#[derive(Debug)]
pub struct Stream {
    inner: TcpStream,
}

impl Stream {
    /// Reports whether the stream authenticates and encrypts its traffic.
    #[must_use]
    pub const fn is_secure(&self) -> bool {
        false
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

/// An error while opening a network connection.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConnectError {
    /// Hostname resolution failed.
    #[error("failed to resolve {host}:{port}: {source}")]
    Resolve {
        /// The hostname that could not be resolved.
        host: String,
        /// The service port.
        port: u16,
        /// The resolver error.
        #[source]
        source: io::Error,
    },
    /// Every resolved address failed to connect.
    #[error("TCP connection failed: {0}")]
    Tcp(#[source] io::Error),
}

/// Resolves a host and connects to each address in order until one succeeds.
///
/// This function is unavailable on `wasm32-wasip1`, which cannot open outbound sockets.
///
/// # Errors
///
/// Returns the resolver error, the last TCP error, or an error explaining that
/// the selected security mode is unavailable.
#[cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]
pub async fn connect(host: &str, port: u16, security: &Security) -> Result<Stream, ConnectError> {
    let Security::Plaintext = security;
    let addresses = resolve(host, port).await?;
    let mut last_error = io::Error::new(io::ErrorKind::NotFound, "hostname resolved no addresses");
    for address in addresses {
        match connect_address(address).await {
            Ok(inner) => return Ok(Stream { inner }),
            Err(error) => last_error = error,
        }
    }
    Err(ConnectError::Tcp(last_error))
}

#[cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]
async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, ConnectError> {
    #[cfg(not(target_os = "wasi"))]
    let result = tokio::net::lookup_host((host, port))
        .await
        .map(Iterator::collect);

    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    let result = {
        use std::net::ToSocketAddrs;
        // Tokio has no WASI hostname resolver, and connect performs this once.
        (host, port).to_socket_addrs().map(Iterator::collect)
    };

    result.map_err(|source| ConnectError::Resolve {
        host: host.to_owned(),
        port,
        source,
    })
}

#[cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]
async fn connect_address(address: SocketAddr) -> io::Result<TcpStream> {
    let stream = TcpStream::connect(address).await?;
    #[cfg(not(target_os = "wasi"))]
    stream.set_nodelay(true)?;
    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    let _ = stream.set_nodelay(true);
    Ok(stream)
}
