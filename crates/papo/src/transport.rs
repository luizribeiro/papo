//! Network connections to IRC servers.

#[cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]
use std::net::SocketAddr;
#[cfg(feature = "tls-ring")]
use std::sync::Arc;
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
#[cfg(feature = "tls-ring")]
use tokio_rustls::client::TlsStream;

/// The security used for a connection.
#[derive(Clone, Debug)]
pub enum Security {
    /// An unencrypted TCP connection.
    Plaintext,
    /// A TLS connection.
    Tls(TlsConfig),
}

/// TLS server-name settings.
#[derive(Clone, Debug, Default)]
pub struct TlsConfig {
    /// A DNS name to authenticate in place of the connection host.
    pub server_name: Option<String>,
}

/// A connected IRC byte stream.
#[derive(Debug)]
pub struct Stream {
    inner: StreamInner,
}

#[derive(Debug)]
enum StreamInner {
    Tcp(TcpStream),
    #[cfg(feature = "tls-ring")]
    Tls(Box<TlsStream<TcpStream>>),
}

impl Stream {
    /// Reports whether the stream authenticates and encrypts its traffic.
    #[must_use]
    pub const fn is_secure(&self) -> bool {
        match &self.inner {
            StreamInner::Tcp(_) => false,
            #[cfg(feature = "tls-ring")]
            StreamInner::Tls(_) => true,
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.inner {
            StreamInner::Tcp(stream) => Pin::new(stream).poll_read(context, buffer),
            #[cfg(feature = "tls-ring")]
            StreamInner::Tls(stream) => Pin::new(stream).poll_read(context, buffer),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match &mut self.inner {
            StreamInner::Tcp(stream) => Pin::new(stream).poll_write(context, buffer),
            #[cfg(feature = "tls-ring")]
            StreamInner::Tls(stream) => Pin::new(stream).poll_write(context, buffer),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.inner {
            StreamInner::Tcp(stream) => Pin::new(stream).poll_flush(context),
            #[cfg(feature = "tls-ring")]
            StreamInner::Tls(stream) => Pin::new(stream).poll_flush(context),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.inner {
            StreamInner::Tcp(stream) => Pin::new(stream).poll_shutdown(context),
            #[cfg(feature = "tls-ring")]
            StreamInner::Tls(stream) => Pin::new(stream).poll_shutdown(context),
        }
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
    /// TLS configuration or negotiation failed.
    #[cfg(feature = "tls-ring")]
    #[error("TLS failed: {0}")]
    Tls(#[source] rustls::Error),
    /// The configured TLS server name is invalid.
    #[cfg(feature = "tls-ring")]
    #[error("invalid TLS server name: {0}")]
    InvalidServerName(String),
    /// TLS support was not selected at compile time.
    #[error("TLS support is disabled; enable tls-ring or tls-aws-lc")]
    TlsUnavailable,
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
    let addresses = resolve(host, port).await?;
    #[cfg(feature = "tls-ring")]
    let tls = match security {
        Security::Plaintext => None,
        Security::Tls(config) => Some(tls_connector(host, config)?),
    };
    #[cfg(not(feature = "tls-ring"))]
    if matches!(security, Security::Tls(_)) {
        return Err(ConnectError::TlsUnavailable);
    }
    let mut last_error = ConnectError::Tcp(io::Error::new(
        io::ErrorKind::NotFound,
        "hostname resolved no addresses",
    ));
    for address in addresses {
        match connect_address(address).await {
            Ok(stream) => {
                #[cfg(feature = "tls-ring")]
                if let Some((connector, server_name)) = &tls {
                    match connector.connect(server_name.clone(), stream).await {
                        Ok(stream) => {
                            return Ok(Stream {
                                inner: StreamInner::Tls(Box::new(stream)),
                            });
                        }
                        Err(error) => {
                            last_error = rustls_error(&error)
                                .map_or_else(|| ConnectError::Tcp(error), ConnectError::Tls);
                            continue;
                        }
                    }
                }
                return Ok(Stream {
                    inner: StreamInner::Tcp(stream),
                });
            }
            Err(error) => last_error = ConnectError::Tcp(error),
        }
    }
    Err(last_error)
}

#[cfg(feature = "tls-ring")]
fn tls_connector(
    host: &str,
    settings: &TlsConfig,
) -> Result<
    (
        tokio_rustls::TlsConnector,
        rustls::pki_types::ServerName<'static>,
    ),
    ConnectError,
> {
    use rustls_platform_verifier::BuilderVerifierExt;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(ConnectError::Tls)?
        .with_platform_verifier()
        .map_err(ConnectError::Tls)?
        .with_no_client_auth();
    config.enable_sni = true;
    let name = settings.server_name.as_deref().unwrap_or(host).to_owned();
    let server_name = rustls::pki_types::ServerName::try_from(name.clone())
        .map_err(|_| ConnectError::InvalidServerName(name))?;
    Ok((
        tokio_rustls::TlsConnector::from(Arc::new(config)),
        server_name,
    ))
}

#[cfg(feature = "tls-ring")]
fn rustls_error(error: &io::Error) -> Option<rustls::Error> {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<rustls::Error>())
        .cloned()
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
