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

/// Stable classification of a server refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Refusal {
    /// The message could not be sent to the channel (`404`).
    CannotSendToChannel,
    /// The session has joined too many channels (`405`).
    TooManyChannels,
    /// A command requiring text received none (`412`).
    NoTextToSend,
    /// An input line exceeded the server limit (`417`).
    InputTooLong,
    /// The server does not recognize the command (`421`).
    UnknownCommand,
    /// A nickname collided during a network split (`436`).
    NickCollision,
    /// A nickname or channel is temporarily unavailable (`437`).
    NickUnavailable,
    /// A channel mask is malformed (`476`).
    BadChannelMask,
    /// The channel accepts only secure connections (`489`).
    SecureOnlyChannel,
    /// The host is not authorized for operator access (`491`).
    NoOperHost,
    /// A user mode flag is unknown (`501`).
    UnknownUserMode,
    /// The session tried to change another user's modes (`502`).
    UsersDontMatch,
    /// A channel mode parameter is invalid (`696`).
    InvalidModeParameter,
    /// The server is temporarily too busy (`263`).
    TryAgain,
    /// A refusal without a more specific classification.
    ///
    /// A later minor release may refine a code currently classified as `Other`.
    Other,
}

impl Refusal {
    pub(crate) fn from_core(kind: papo_core::RefusalKind) -> Self {
        match kind {
            papo_core::RefusalKind::CannotSendToChannel => Self::CannotSendToChannel,
            papo_core::RefusalKind::TooManyChannels => Self::TooManyChannels,
            papo_core::RefusalKind::NoTextToSend => Self::NoTextToSend,
            papo_core::RefusalKind::InputTooLong => Self::InputTooLong,
            papo_core::RefusalKind::UnknownCommand => Self::UnknownCommand,
            papo_core::RefusalKind::NickCollision => Self::NickCollision,
            papo_core::RefusalKind::NickUnavailable => Self::NickUnavailable,
            papo_core::RefusalKind::BadChannelMask => Self::BadChannelMask,
            papo_core::RefusalKind::SecureOnlyChannel => Self::SecureOnlyChannel,
            papo_core::RefusalKind::NoOperHost => Self::NoOperHost,
            papo_core::RefusalKind::UnknownUserMode => Self::UnknownUserMode,
            papo_core::RefusalKind::UsersDontMatch => Self::UsersDontMatch,
            papo_core::RefusalKind::InvalidModeParam => Self::InvalidModeParameter,
            papo_core::RefusalKind::TryAgain => Self::TryAgain,
            _ => Self::Other,
        }
    }
}

/// A server refusal with its wire-level diagnostic context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerError {
    kind: Refusal,
    command: String,
    code: String,
    description: String,
    params: Vec<String>,
}

impl ServerError {
    /// Returns the stable refusal category.
    #[must_use]
    pub const fn kind(&self) -> Refusal {
        self.kind
    }

    /// Returns the command named by a standard reply, or an empty string for a numeric.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Returns the standard-reply code or three-digit numeric.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the server's human-readable description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns additional server-supplied parameters in wire order.
    #[must_use]
    pub fn params(&self) -> &[String] {
        &self.params
    }

    pub(crate) fn from_core(error: papo_core::ServerError) -> Self {
        let kind = Refusal::from_core(error.refusal_kind());
        match error {
            papo_core::ServerError::Standard {
                command,
                code,
                context,
                description,
            } => Self {
                kind,
                command,
                code,
                description,
                params: context,
            },
            papo_core::ServerError::Numeric { numeric, params } => Self {
                kind,
                command: String::new(),
                code: format!("{:03}", numeric.code()),
                description: params.last().cloned().unwrap_or_default(),
                params,
            },
            other => Self {
                kind: Refusal::Other,
                command: String::new(),
                code: String::new(),
                description: format!("{other:?}"),
                params: Vec::new(),
            },
        }
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.command.is_empty() {
            write!(
                formatter,
                "server refusal {}: {}",
                self.code, self.description
            )
        } else {
            write!(
                formatter,
                "{} {}: {}",
                self.command, self.code, self.description
            )
        }
    }
}

impl std::error::Error for ServerError {}

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
    #[error("the IRC server rejected the request: {0}")]
    Server(ServerError),
}

impl Error {
    pub(crate) fn server(error: papo_core::ServerError) -> Self {
        Self::Server(ServerError::from_core(error))
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{ConnectError, ConnectErrorKind, Error, Refusal, ServerError};

    #[test]
    fn server_errors_hide_core_shapes_behind_accessors() {
        let error = ServerError::from_core(papo_core::ServerError::Numeric {
            numeric: papo_proto::Numeric::ERR_CANNOTSENDTOCHAN,
            params: vec!["papo-driver".into(), "#rust".into(), "Cannot send".into()],
        });
        assert_eq!(error.kind(), Refusal::CannotSendToChannel);
        assert_eq!(error.command(), "");
        assert_eq!(error.code(), "404");
        assert_eq!(error.description(), "Cannot send");
        assert_eq!(error.params()[1], "#rust");
        assert_eq!(error.to_string(), "server refusal 404: Cannot send");
    }

    #[test]
    fn refusal_mapping_preserves_every_classified_numeric() {
        use papo_core::RefusalKind as Core;
        let cases = [
            (Core::CannotSendToChannel, Refusal::CannotSendToChannel),
            (Core::TooManyChannels, Refusal::TooManyChannels),
            (Core::NoTextToSend, Refusal::NoTextToSend),
            (Core::InputTooLong, Refusal::InputTooLong),
            (Core::UnknownCommand, Refusal::UnknownCommand),
            (Core::NickCollision, Refusal::NickCollision),
            (Core::NickUnavailable, Refusal::NickUnavailable),
            (Core::BadChannelMask, Refusal::BadChannelMask),
            (Core::SecureOnlyChannel, Refusal::SecureOnlyChannel),
            (Core::NoOperHost, Refusal::NoOperHost),
            (Core::UnknownUserMode, Refusal::UnknownUserMode),
            (Core::UsersDontMatch, Refusal::UsersDontMatch),
            (Core::InvalidModeParam, Refusal::InvalidModeParameter),
            (Core::TryAgain, Refusal::TryAgain),
            (Core::Other, Refusal::Other),
        ];
        for (core, public) in cases {
            assert_eq!(Refusal::from_core(core), public);
        }
    }

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
