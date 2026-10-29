use std::sync::Arc;

use zeroize::Zeroize;

/// The roots trusted by a TLS connection.
#[derive(Clone, Debug)]
pub(crate) enum TlsRoots {
    /// Roots supplied by the operating system, or `WebPKI` roots on WASI.
    Platform,
    /// Only the certificates in the supplied PEM document.
    Ca(Vec<u8>),
    /// Platform roots plus the certificates in the supplied PEM document.
    PlatformAndCa(Vec<u8>),
}

/// A client certificate chain and its private key, both PEM encoded.
#[derive(Clone, Debug)]
pub(crate) struct ClientCertificate {
    pub(super) certificate_chain: CertificateChain,
    pub(super) private_key: PrivateKey,
}

/// A PEM-encoded TLS client certificate chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateChain(pub(super) Vec<u8>);

impl CertificateChain {
    /// Wraps PEM-encoded client certificate data.
    #[must_use]
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(value.into())
    }
}

/// A PEM-encoded TLS private key whose debug representation is redacted.
#[derive(Clone)]
pub struct PrivateKey(pub(super) Arc<PrivateKeyBytes>);

pub(super) struct PrivateKeyBytes(pub(super) Vec<u8>);

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
    /// Wraps PEM-encoded private-key data.
    #[must_use]
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(Arc::new(PrivateKeyBytes(value.into())))
    }

    pub(super) fn expose(&self) -> &[u8] {
        &self.0.0
    }
}

impl ClientCertificate {
    pub(crate) fn new(certificate_chain: CertificateChain, private_key: PrivateKey) -> Self {
        Self {
            certificate_chain,
            private_key,
        }
    }
}

/// TLS trust, identity, and server-name settings.
#[derive(Clone, Debug)]
pub(crate) struct TlsConfig {
    pub(crate) roots: TlsRoots,
    pub(crate) client_certificate: Option<ClientCertificate>,
    pub(crate) server_name: Option<String>,
    pub(crate) prepared: Option<Arc<rustls::ClientConfig>>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            roots: TlsRoots::Platform,
            client_certificate: None,
            server_name: None,
            prepared: None,
        }
    }
}

impl TlsConfig {
    pub(crate) fn has_custom_settings(&self) -> bool {
        !matches!(self.roots, TlsRoots::Platform)
            || self.client_certificate.is_some()
            || self.server_name.is_some()
    }
}
