use std::{fmt, io, sync::Arc};

use papo_proto::BuildError;
use thiserror::Error;

use crate::InvalidInput;

/// Broad classification of a connection-opening failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectErrorKind {
    /// The server name could not be resolved.
    Resolve,
    /// The remote endpoint refused the connection.
    Refused,
    /// TLS negotiation failed.
    Tls,
    /// The server certificate was rejected.
    Certificate,
    /// TLS settings or credentials were invalid or unavailable.
    TlsConfig,
    /// WebSocket parsing or negotiation failed.
    WebSocket,
    /// Another connection I/O operation failed.
    Io,
}

/// An opaque, cloneable error from opening a network connection.
#[derive(Clone, Debug)]
pub struct ConnectError(Arc<crate::transport::ConnectError>);

impl ConnectError {
    /// Returns the stable category of this failure.
    #[must_use]
    pub fn kind(&self) -> ConnectErrorKind {
        match self.0.as_ref() {
            crate::transport::ConnectError::Resolve { .. } => ConnectErrorKind::Resolve,
            crate::transport::ConnectError::Tcp(error)
                if error.kind() == io::ErrorKind::ConnectionRefused =>
            {
                ConnectErrorKind::Refused
            }
            crate::transport::ConnectError::Tcp(_) => ConnectErrorKind::Io,
            #[cfg(tls)]
            crate::transport::ConnectError::Tls(rustls::Error::InvalidCertificate(_)) => {
                ConnectErrorKind::Certificate
            }
            #[cfg(tls)]
            crate::transport::ConnectError::Tls(_) => ConnectErrorKind::Tls,
            #[cfg(tls)]
            crate::transport::ConnectError::InvalidServerName(_)
            | crate::transport::ConnectError::InvalidTlsConfiguration(_) => {
                ConnectErrorKind::TlsConfig
            }
            crate::transport::ConnectError::TlsUnavailable => ConnectErrorKind::TlsConfig,
            #[cfg(all(feature = "websocket", sockets))]
            crate::transport::ConnectError::WebSocket(_) => ConnectErrorKind::WebSocket,
        }
    }
}

impl From<crate::transport::ConnectError> for ConnectError {
    fn from(error: crate::transport::ConnectError) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

/// An error produced by the asynchronous IRC client.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The configured server address is invalid.
    #[error("invalid server address: {0}")]
    InvalidInput(#[from] InvalidInput),
    /// Reading from or writing to the connection failed.
    #[error("connection I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Opening the connection failed.
    #[error("connection failed: {0}")]
    Connect(#[from] ConnectError),
    /// A server requested consecutive STS upgrades without registering.
    #[error("repeated STS upgrade for {host} without registration")]
    StsUpgradeLoop {
        /// The hostname whose server repeated the upgrade.
        host: String,
    },
    /// The requested transport is not available in this build.
    #[error("unsupported transport: {0}")]
    Unsupported(&'static str),
    /// An outbound IRC message is not valid on the wire.
    #[error("invalid IRC message: {0}")]
    Build(#[from] BuildError),
    /// The connection already has 256 requests awaiting replies.
    #[error("too many IRC requests are awaiting replies")]
    RequestLimit,
    /// The outgoing chat text contains no nonempty lines.
    #[error("chat text is empty")]
    EmptyText,
    /// Registration ended before the server welcomed the client.
    #[error("IRC registration failed: {0}")]
    Registration(String),
    /// The server disconnected before the requested operation completed.
    #[error("the IRC connection closed")]
    Disconnected,
    /// An operation did not complete before its deadline.
    #[error("the IRC operation timed out")]
    Timeout,
    /// The IRC server rejected a request.
    #[error("the IRC server rejected the request: {0:?}")]
    Server(papo_core::ServerError),
    /// The IRC server rejected account registration or verification.
    #[error("account registration failed: {0:?}")]
    AccountRegistration(papo_core::AccountRegistrationError),
    /// The IRC server rejected a metadata operation.
    #[error("metadata operation failed: {0:?}")]
    Metadata(papo_core::MetadataError),
    /// The IRC server rejected a channel rename.
    #[error("channel rename failed: {0:?}")]
    ChannelRename(papo_core::ChannelRenameError),
    /// The IRC server rejected a relayed message.
    #[error("relay message failed: {0:?}")]
    Relay(papo_core::RelayError),
    /// The IRC server rejected a Web Push operation.
    #[error("Web Push operation failed: {0:?}")]
    WebPush(papo_core::WebPushError),
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{ConnectError, ConnectErrorKind, Error};

    #[test]
    fn connection_error_exposes_only_its_stable_kind() {
        let error = ConnectError::from(crate::transport::ConnectError::Tcp(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "recorded refusal",
        )));
        assert_eq!(error.kind(), ConnectErrorKind::Refused);
        assert_eq!(error.to_string(), "TCP connection failed: recorded refusal");
        assert!(std::error::Error::source(&error).is_some());
        assert_eq!(error.clone().kind(), ConnectErrorKind::Refused);
    }

    #[test]
    fn io_error_keeps_its_source() {
        let error = Error::from(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        ));
        assert_eq!(
            error.to_string(),
            "connection I/O failed: connection reset by peer"
        );
    }

    #[test]
    fn terminal_errors_have_distinct_messages() {
        assert_eq!(Error::Disconnected.to_string(), "the IRC connection closed");
        assert_eq!(Error::Timeout.to_string(), "the IRC operation timed out");
    }
}
