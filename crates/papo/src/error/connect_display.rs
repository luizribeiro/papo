use std::fmt;

use super::{ConnectError, ConnectErrorKind, displays_server_reason};

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ConnectErrorKind::CapabilityUnavailable {
            return self.source.fmt(formatter);
        }
        let message = match self.kind {
            #[cfg(sockets)]
            ConnectErrorKind::HostNotFound => "the server host could not be found",
            #[cfg(sockets)]
            ConnectErrorKind::Refused => "the server refused the connection",
            #[cfg(sockets)]
            ConnectErrorKind::Unreachable => "the server could not be reached",
            ConnectErrorKind::TimedOut => "the connection attempt timed out",
            #[cfg(tls)]
            ConnectErrorKind::CertificateRejected => "the server certificate was rejected",
            ConnectErrorKind::HandshakeFailed => "the connection handshake failed",
            ConnectErrorKind::Sasl => "SASL authentication failed",
            ConnectErrorKind::SaslUnavailable => "SASL authentication is unavailable",
            ConnectErrorKind::CapabilityUnavailable => unreachable!(),
            ConnectErrorKind::AccountRequired => "the server requires an authenticated account",
            ConnectErrorKind::PasswordIncorrect => "the server rejected the connection password",
            ConnectErrorKind::Banned => "the server banned this connection",
            ConnectErrorKind::WebIrcRejected => "the server rejected the WEBIRC gateway",
            ConnectErrorKind::StsUpgradeLoop => "the server repeatedly requested an STS upgrade",
            ConnectErrorKind::Server => "the server rejected the connection",
            ConnectErrorKind::WelcomeTimedOut => "IRC login timed out",
            ConnectErrorKind::Other => "the connection could not be completed",
        };
        formatter.write_str(message)?;
        if displays_server_reason(self.kind) {
            let reason = self.source.to_string();
            if !reason.is_empty() {
                write!(formatter, ": {reason}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        (!displays_server_reason(self.kind)).then_some(self.source.as_ref())
    }
}
