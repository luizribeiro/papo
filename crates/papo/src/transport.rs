//! Network connections to IRC servers.

#[cfg(sockets)]
use std::net::SocketAddr;
#[cfg(tls)]
use std::sync::Arc;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
#[cfg(sockets)]
use tokio::net::TcpStream;
#[cfg(tls)]
use tokio_rustls::client::TlsStream;
use zeroize::Zeroize;

#[cfg(all(feature = "websocket", sockets))]
mod websocket;
#[cfg(browser)]
#[path = "transport/websocket_browser.rs"]
mod websocket_browser;
#[cfg(all(test, not(browser)))]
#[path = "transport/websocket_browser_native.rs"]
mod websocket_browser_native;
#[cfg(all(test, not(browser)))]
#[path = "transport/websocket_browser/pure.rs"]
mod websocket_browser_pure;

#[cfg(all(test, not(browser)))]
mod live;

/// The security used for a connection.
#[derive(Clone, Debug)]
pub enum Security {
    /// An unencrypted TCP connection.
    Plaintext,
    /// A TLS connection.
    Tls(TlsConfig),
}

/// The roots trusted by a TLS connection.
#[derive(Clone, Debug)]
pub enum TlsRoots {
    /// Roots supplied by the operating system, or `WebPKI` roots on WASI.
    Platform,
    /// Only the certificates in the supplied PEM document.
    Ca(Vec<u8>),
    /// Platform roots plus the certificates in the supplied PEM document.
    PlatformAndCa(Vec<u8>),
}

/// A client certificate chain and its private key, both PEM encoded.
#[derive(Clone, Debug)]
pub struct ClientCertificate {
    certificate_chain: Vec<u8>,
    private_key: PrivateKey,
}

#[derive(Clone)]
struct PrivateKey(std::sync::Arc<PrivateKeyBytes>);

struct PrivateKeyBytes(Vec<u8>);

impl Drop for PrivateKeyBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl PrivateKey {
    fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(std::sync::Arc::new(PrivateKeyBytes(value.into())))
    }

    fn expose(&self) -> &[u8] {
        &self.0.0
    }
}

impl ClientCertificate {
    /// Creates client credentials from PEM-encoded certificate and key data.
    #[must_use]
    pub fn new(certificate_chain: impl Into<Vec<u8>>, private_key: impl Into<Vec<u8>>) -> Self {
        Self {
            certificate_chain: certificate_chain.into(),
            private_key: PrivateKey::new(private_key),
        }
    }
}

/// TLS trust, identity, and server-name settings.
///
/// TLS 1.2 remains enabled alongside TLS 1.3 because Ergo's default minimum
/// and many IRC networks still accept 1.2. Rustls offers only secure TLS 1.2
/// cipher suites.
#[derive(Clone, Debug)]
pub struct TlsConfig {
    /// The roots used to authenticate the server.
    pub roots: TlsRoots,
    /// A certificate presented when the server requests client authentication.
    pub client_certificate: Option<ClientCertificate>,
    /// A DNS name to authenticate in place of the connection host.
    pub server_name: Option<String>,
    #[cfg(tls)]
    prepared: Option<Arc<rustls::ClientConfig>>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            roots: TlsRoots::Platform,
            client_certificate: None,
            server_name: None,
            #[cfg(tls)]
            prepared: None,
        }
    }
}

/// A connected IRC byte stream.
#[derive(Debug)]
pub struct Stream {
    inner: StreamInner,
}

#[derive(Debug)]
enum StreamInner {
    #[cfg(sockets)]
    Tcp(TcpStream),
    #[cfg(tls)]
    Tls(Box<TlsStream<TcpStream>>),
    #[cfg(all(feature = "websocket", sockets))]
    WebSocket(Box<websocket::WebSocket>),
    #[cfg(browser)]
    BrowserWebSocket(Box<websocket_browser::WebSocket>),
    #[cfg(all(test, feature = "websocket"))]
    BrokenPeer,
    #[cfg(not(sockets))]
    #[allow(dead_code)]
    Unavailable,
}

impl Stream {
    /// Reports whether the stream authenticates and encrypts its traffic.
    #[must_use]
    #[cfg(test)]
    pub const fn is_secure(&self) -> bool {
        match &self.inner {
            #[cfg(sockets)]
            StreamInner::Tcp(_) => false,
            #[cfg(tls)]
            StreamInner::Tls(_) => true,
            #[cfg(all(feature = "websocket", sockets))]
            StreamInner::WebSocket(stream) => stream.is_secure(),
            #[cfg(browser)]
            StreamInner::BrowserWebSocket(stream) => stream.is_secure(),
            #[cfg(all(test, feature = "websocket"))]
            StreamInner::BrokenPeer => false,
            #[cfg(not(sockets))]
            StreamInner::Unavailable => false,
        }
    }

    /// Returns the negotiated WebSocket subprotocol, if this is a WebSocket stream.
    #[must_use]
    #[cfg(test)]
    pub fn websocket_subprotocol(&self) -> Option<&str> {
        match &self.inner {
            #[cfg(all(feature = "websocket", sockets))]
            StreamInner::WebSocket(stream) => stream.subprotocol(),
            #[cfg(browser)]
            StreamInner::BrowserWebSocket(stream) => stream.subprotocol(),
            _ => None,
        }
    }
}

#[cfg_attr(not(sockets), allow(unused_variables))]
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut self.inner {
            #[cfg(sockets)]
            StreamInner::Tcp(stream) => Pin::new(stream).poll_read(context, buffer),
            #[cfg(tls)]
            StreamInner::Tls(stream) => Pin::new(stream).poll_read(context, buffer),
            #[cfg(all(feature = "websocket", sockets))]
            StreamInner::WebSocket(stream) => Pin::new(stream.as_mut()).poll_read(context, buffer),
            #[cfg(browser)]
            StreamInner::BrowserWebSocket(stream) => {
                Pin::new(stream.as_mut()).poll_read(context, buffer)
            }
            #[cfg(all(test, feature = "websocket"))]
            StreamInner::BrokenPeer => Poll::Ready(Err(broken_peer())),
            #[cfg(not(sockets))]
            StreamInner::Unavailable => Poll::Ready(Err(transport_unavailable())),
        }
    }
}

#[cfg_attr(not(sockets), allow(unused_variables))]
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match &mut self.inner {
            #[cfg(sockets)]
            StreamInner::Tcp(stream) => Pin::new(stream).poll_write(context, buffer),
            #[cfg(tls)]
            StreamInner::Tls(stream) => Pin::new(stream).poll_write(context, buffer),
            #[cfg(all(feature = "websocket", sockets))]
            StreamInner::WebSocket(stream) => Pin::new(stream.as_mut()).poll_write(context, buffer),
            #[cfg(browser)]
            StreamInner::BrowserWebSocket(stream) => {
                Pin::new(stream.as_mut()).poll_write(context, buffer)
            }
            #[cfg(all(test, feature = "websocket"))]
            StreamInner::BrokenPeer => Poll::Ready(Err(broken_peer())),
            #[cfg(not(sockets))]
            StreamInner::Unavailable => Poll::Ready(Err(transport_unavailable())),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.inner {
            #[cfg(sockets)]
            StreamInner::Tcp(stream) => Pin::new(stream).poll_flush(context),
            #[cfg(tls)]
            StreamInner::Tls(stream) => Pin::new(stream).poll_flush(context),
            #[cfg(all(feature = "websocket", sockets))]
            StreamInner::WebSocket(stream) => Pin::new(stream.as_mut()).poll_flush(context),
            #[cfg(browser)]
            StreamInner::BrowserWebSocket(stream) => Pin::new(stream.as_mut()).poll_flush(context),
            #[cfg(all(test, feature = "websocket"))]
            StreamInner::BrokenPeer => Poll::Ready(Err(broken_peer())),
            #[cfg(not(sockets))]
            StreamInner::Unavailable => Poll::Ready(Err(transport_unavailable())),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.inner {
            #[cfg(sockets)]
            StreamInner::Tcp(stream) => Pin::new(stream).poll_shutdown(context),
            #[cfg(tls)]
            StreamInner::Tls(stream) => Pin::new(stream).poll_shutdown(context),
            #[cfg(all(feature = "websocket", sockets))]
            StreamInner::WebSocket(stream) => Pin::new(stream.as_mut()).poll_shutdown(context),
            #[cfg(browser)]
            StreamInner::BrowserWebSocket(stream) => {
                Pin::new(stream.as_mut()).poll_shutdown(context)
            }
            #[cfg(all(test, feature = "websocket"))]
            StreamInner::BrokenPeer => Poll::Ready(Err(broken_peer())),
            #[cfg(not(sockets))]
            StreamInner::Unavailable => Poll::Ready(Err(transport_unavailable())),
        }
    }
}

#[cfg(any(not(sockets), test))]
fn transport_unavailable() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "transport is unavailable")
}

#[cfg(all(test, feature = "websocket"))]
fn broken_peer() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "broken WebSocket peer")
}

/// An error while opening a network connection.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ConnectError {
    /// This build or target cannot open the requested transport.
    #[allow(dead_code)]
    #[error("{0} is unavailable")]
    Unavailable(&'static str),
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
    #[cfg(tls)]
    #[error("TLS failed: {0}")]
    Tls(#[source] rustls::Error),
    /// The configured TLS server name is invalid.
    #[cfg(tls)]
    #[error("invalid TLS server name: {0}")]
    InvalidServerName(String),
    /// PEM-encoded TLS configuration was invalid or empty.
    #[cfg(tls)]
    #[error("invalid TLS configuration: {0}")]
    InvalidTlsConfiguration(String),
    /// TLS support was not selected at compile time.
    #[cfg_attr(tls, allow(dead_code))]
    #[error("TLS support is disabled; enable tls-ring or tls-aws-lc")]
    TlsUnavailable,
    /// WebSocket URL parsing or negotiation failed.
    #[cfg(all(feature = "websocket", sockets))]
    #[error("WebSocket failed: {0}")]
    WebSocket(#[source] tokio_tungstenite::tungstenite::Error),
    /// The browser rejected or closed a WebSocket connection.
    #[cfg(browser)]
    #[error("WebSocket failed: {0}")]
    BrowserWebSocket(String),
}

/// Opens a WebSocket connection over the existing TCP and TLS transports.
///
/// Binary `IRCv3` messages are preferred over text messages. TLS settings are
/// used for `wss` URLs; when omitted, the platform trust configuration applies.
///
/// # Errors
///
/// Returns an error for an invalid URL or failed TCP, TLS, or WebSocket handshake.
#[cfg(all(feature = "websocket", sockets))]
pub(crate) async fn connect_websocket(
    url: &str,
    tls: Option<&TlsConfig>,
) -> Result<Stream, ConnectError> {
    websocket::connect(url, tls).await.map(|stream| Stream {
        inner: StreamInner::WebSocket(Box::new(stream)),
    })
}

/// Opens a browser WebSocket connection.
///
/// # Errors
///
/// Returns an error when the browser rejects the URL or the opening handshake fails.
#[cfg(browser)]
pub async fn connect_websocket(
    url: &str,
    _tls: Option<&TlsConfig>,
) -> Result<Stream, ConnectError> {
    websocket_browser::WebSocket::connect(url)
        .await
        .map(|stream| Stream {
            inner: StreamInner::BrowserWebSocket(Box::new(stream)),
        })
        .map_err(ConnectError::BrowserWebSocket)
}

/// Resolves a host and connects to each address in order until one succeeds.
///
/// This function is unavailable on `wasm32-wasip1`, which cannot open outbound sockets.
///
/// # Errors
///
/// Returns the resolver error, the last TCP error, or an error explaining that
/// the selected security mode is unavailable.
#[cfg(sockets)]
pub(crate) async fn connect(
    host: &str,
    port: u16,
    security: &Security,
) -> Result<Stream, ConnectError> {
    #[cfg(not(tls))]
    if matches!(security, Security::Tls(_)) {
        return Err(ConnectError::TlsUnavailable);
    }
    let addresses = resolve(host, port).await?;
    #[cfg(tls)]
    let tls = match security {
        Security::Plaintext => None,
        Security::Tls(config) => Some(tls_connector(host, config)?),
    };
    let mut last_error = ConnectError::Tcp(io::Error::new(
        io::ErrorKind::NotFound,
        "hostname resolved no addresses",
    ));
    for address in addresses {
        match connect_address(address).await {
            Ok(stream) => {
                #[cfg(tls)]
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

#[cfg(tls)]
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
    let config = settings
        .prepared
        .clone()
        .map_or_else(|| build_tls_client_config(settings), Ok)?;
    let name = settings.server_name.as_deref().unwrap_or(host).to_owned();
    let server_name = rustls::pki_types::ServerName::try_from(name.clone())
        .map_err(|_| ConnectError::InvalidServerName(name))?;
    Ok((tokio_rustls::TlsConnector::from(config), server_name))
}

#[cfg(tls)]
fn build_tls_client_config(
    settings: &TlsConfig,
) -> Result<Arc<rustls::ClientConfig>, ConnectError> {
    use rustls::pki_types::{PrivateKeyDer, pem::PemObject};
    use rustls_platform_verifier::BuilderVerifierExt;

    let provider = tls_provider();
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(ConnectError::Tls)?;
    let builder = match &settings.roots {
        TlsRoots::Platform => builder
            .with_platform_verifier()
            .map_err(ConnectError::Tls)?,
        TlsRoots::Ca(pem) => {
            let mut roots = rustls::RootCertStore::empty();
            for certificate in certificates(pem)? {
                roots.add(certificate).map_err(ConnectError::Tls)?;
            }
            builder.with_root_certificates(roots)
        }
        TlsRoots::PlatformAndCa(pem) => {
            let verifier = rustls_platform_verifier::Verifier::new_with_extra_roots(
                certificates(pem)?,
                provider,
            )
            .map_err(ConnectError::Tls)?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
        }
    };
    let mut config = if let Some(identity) = &settings.client_certificate {
        let chain = certificates(&identity.certificate_chain)?;
        let key = PrivateKeyDer::from_pem_slice(identity.private_key.expose())
            .map_err(|_| invalid_pem("client private key"))?;
        builder
            .with_client_auth_cert(chain, key)
            .map_err(ConnectError::Tls)?
    } else {
        builder.with_no_client_auth()
    };
    config.enable_sni = true;
    Ok(Arc::new(config))
}

#[cfg(tls)]
pub(crate) fn prepare_tls_config(host: &str, settings: &mut TlsConfig) -> Result<(), String> {
    let config = build_tls_client_config(settings).map_err(|error| error.to_string())?;
    let name = settings.server_name.as_deref().unwrap_or(host).to_owned();
    rustls::pki_types::ServerName::try_from(name.clone())
        .map_err(|_| ConnectError::InvalidServerName(name).to_string())?;
    settings.prepared = Some(config);
    Ok(())
}

#[cfg(all(tls, feature = "tls-ring"))]
fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[cfg(all(tls, not(feature = "tls-ring"), feature = "tls-aws-lc"))]
fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

#[cfg(tls)]
fn certificates(
    pem: &[u8],
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, ConnectError> {
    use rustls::pki_types::{CertificateDer, pem::PemObject};

    let certificates = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid_pem("certificate"))?;
    if certificates.is_empty() {
        return Err(ConnectError::InvalidTlsConfiguration(
            "certificate PEM contains no certificates".into(),
        ));
    }
    Ok(certificates)
}

#[cfg(tls)]
fn invalid_pem(kind: &str) -> ConnectError {
    ConnectError::InvalidTlsConfiguration(format!("invalid {kind} PEM"))
}

#[cfg(tls)]
fn rustls_error(error: &io::Error) -> Option<rustls::Error> {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<rustls::Error>())
        .cloned()
}

#[cfg(sockets)]
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

#[cfg(sockets)]
async fn connect_address(address: SocketAddr) -> io::Result<TcpStream> {
    let stream = TcpStream::connect(address).await?;
    #[cfg(not(target_os = "wasi"))]
    stream.set_nodelay(true)?;
    #[cfg(all(target_os = "wasi", not(target_env = "p1")))]
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

#[cfg(all(test, not(browser)))]
mod tests {
    use super::{ClientCertificate, TlsConfig};
    #[cfg(tls)]
    use super::{ConnectError, TlsRoots, prepare_tls_config, tls_connector};

    #[cfg(tls)]
    const CLIENT_CERTIFICATE: &[u8] = include_bytes!("../tests/data/client.pem");
    #[cfg(tls)]
    const CLIENT_KEY: &[u8] = include_bytes!("../tests/data/client.key");

    #[test]
    fn client_private_key_is_redacted() {
        let config = TlsConfig {
            client_certificate: Some(ClientCertificate::new(
                b"public certificate".to_vec(),
                b"private key material".to_vec(),
            )),
            ..TlsConfig::default()
        };
        let output = format!("{config:?}");
        assert!(output.contains("certificate_chain"));
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("private key material"));
    }

    #[test]
    fn unavailable_transport_has_a_typed_diagnostic() {
        let error = super::transport_unavailable();
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert_eq!(error.to_string(), "transport is unavailable");
    }

    #[cfg(tls)]
    #[test]
    fn empty_ca_document_is_rejected() {
        let config = TlsConfig {
            roots: TlsRoots::Ca(Vec::new()),
            ..TlsConfig::default()
        };
        assert!(matches!(
            tls_connector("localhost", &config),
            Err(ConnectError::InvalidTlsConfiguration(message))
                if message == "certificate PEM contains no certificates"
        ));
    }

    #[cfg(tls)]
    #[test]
    fn malformed_ca_document_is_rejected() {
        let config = TlsConfig {
            roots: TlsRoots::Ca(
                b"-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n".to_vec(),
            ),
            ..TlsConfig::default()
        };
        assert!(matches!(
            tls_connector("localhost", &config),
            Err(ConnectError::InvalidTlsConfiguration(message))
                if message == "invalid certificate PEM"
        ));
    }

    #[cfg(tls)]
    #[test]
    fn malformed_client_key_error_does_not_echo_the_input() {
        let marker = "unique-private-key-error-marker-4912";
        let config = TlsConfig {
            client_certificate: Some(ClientCertificate::new(
                CLIENT_CERTIFICATE,
                format!("-----BEGIN PRIVATE KEY-----\n{marker}\n-----END PRIVATE KEY-----")
                    .into_bytes(),
            )),
            ..TlsConfig::default()
        };
        let Err(error) = tls_connector("localhost", &config) else {
            panic!("malformed private key was accepted")
        };
        assert_eq!(
            error.to_string(),
            "invalid TLS configuration: invalid client private key PEM"
        );
        assert!(!error.to_string().contains(marker));
    }

    #[cfg(tls)]
    #[test]
    fn prepared_client_identity_is_shared_by_plan_clones() {
        let mut config = TlsConfig {
            client_certificate: Some(ClientCertificate::new(
                CLIENT_CERTIFICATE,
                CLIENT_KEY.to_vec(),
            )),
            ..TlsConfig::default()
        };
        prepare_tls_config("localhost", &mut config).unwrap();
        let cloned = config.clone();
        assert!(std::sync::Arc::ptr_eq(
            &config.client_certificate.as_ref().unwrap().private_key.0,
            &cloned.client_certificate.as_ref().unwrap().private_key.0
        ));
        assert!(std::sync::Arc::ptr_eq(
            config.prepared.as_ref().unwrap(),
            cloned.prepared.as_ref().unwrap()
        ));
    }

    #[cfg(all(feature = "tls-ring", feature = "tls-aws-lc"))]
    #[test]
    fn ring_wins_when_both_providers_are_enabled() {
        let groups = |provider: &rustls::crypto::CryptoProvider| {
            provider
                .kx_groups
                .iter()
                .map(|group| group.name())
                .collect::<Vec<_>>()
        };
        let selected = super::tls_provider();
        let ring = rustls::crypto::ring::default_provider();
        let aws_lc = rustls::crypto::aws_lc_rs::default_provider();
        assert_ne!(groups(&ring), groups(&aws_lc));
        assert_eq!(groups(&selected), groups(&ring));
    }
}
