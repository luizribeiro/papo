use super::Builder;
use crate::transport::{ClientCertificate, TlsRoots};

impl Builder {
    /// Trusts only the certificates in a PEM document.
    ///
    /// This and [`Self::tls_extra_ca_pem`] replace the previously configured roots; the last call
    /// wins.
    ///
    /// An IP endpoint can use a private CA while authenticating the DNS name in the certificate:
    ///
    /// ```no_run
    /// # async fn connect() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::fs;
    ///
    /// let ca_pem = fs::read("private-ca.pem")?;
    /// let (_client, _events) = papo::Client::builder("ircs://127.0.0.1:6697", "papo-client")
    ///     .tls_ca_pem(ca_pem)
    ///     .tls_server_name("irc.internal.example")
    ///     .connect()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn tls_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::Ca(pem.into());
        self
    }

    /// Adds certificates from a PEM document to the platform trust roots.
    ///
    /// This and [`Self::tls_ca_pem`] replace the previously configured roots; the last call wins.
    ///
    /// ```
    /// # _ = papo::Client::builder("ircs://irc.example.net", "papo-client").tls_extra_ca_pem(Vec::new());
    /// ```
    pub fn tls_extra_ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.tls.roots = TlsRoots::PlatformAndCa(pem.into());
        self
    }

    /// Sets a PEM-encoded TLS client certificate chain and private key.
    ///
    /// ```compile_fail
    /// # use papo::{CertificateChain, Client, PrivateKey};
    /// let certificate = CertificateChain::new(Vec::new());
    /// let key = PrivateKey::new(Vec::new());
    /// let _ = Client::builder("ircs://irc.example.net", "papo-client")
    ///     .tls_client_cert(key, certificate);
    /// ```
    ///
    /// ```no_run
    /// # fn configure(cert: &std::path::Path, key: &std::path::Path) -> std::io::Result<papo::Builder> {
    /// use papo::{CertificateChain, PrivateKey};
    /// use std::fs;
    /// let builder = papo::Client::builder("ircs://irc.example.net", "papo-client")
    ///     .tls_client_cert(
    ///         CertificateChain::new(fs::read(cert)?),
    ///         PrivateKey::new(fs::read(key)?),
    ///     );
    /// # Ok(builder)
    /// # }
    /// ```
    pub fn tls_client_cert(
        mut self,
        certificate: crate::CertificateChain,
        key: crate::PrivateKey,
    ) -> Self {
        self.tls.client_certificate = Some(ClientCertificate::new(certificate, key));
        self
    }

    /// Overrides the DNS name authenticated by TLS.
    ///
    /// This permits connecting to an IP endpoint while verifying a certificate for a DNS name.
    /// See the private-CA example on [`Self::tls_ca_pem`].
    pub fn tls_server_name(mut self, server_name: impl Into<String>) -> Self {
        self.tls.server_name = Some(server_name.into());
        self
    }
}
