use std::io;

use super::ConnectErrorKind;

pub(super) fn classify(error: &crate::transport::ConnectError) -> ConnectErrorKind {
    #[cfg(tls)]
    if matches!(
        error,
        crate::transport::ConnectError::Tls(rustls::Error::InvalidCertificate(_))
    ) {
        return ConnectErrorKind::CertificateRejected;
    }
    #[allow(
        unreachable_patterns,
        reason = "handshake variants are selected by additive transport features"
    )]
    match error {
        crate::transport::ConnectError::Resolve { .. } => ConnectErrorKind::HostNotFound,
        crate::transport::ConnectError::Tcp(error)
            if error.kind() == io::ErrorKind::ConnectionRefused =>
        {
            ConnectErrorKind::Refused
        }
        crate::transport::ConnectError::Tcp(error) if error.kind() == io::ErrorKind::TimedOut => {
            ConnectErrorKind::TimedOut
        }
        crate::transport::ConnectError::Tcp(_) => ConnectErrorKind::Unreachable,
        _ => ConnectErrorKind::HandshakeFailed,
    }
}
