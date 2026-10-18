use std::{fmt, io, sync::Arc, time::Duration};

use thiserror::Error;

use crate::{InvalidInput, Refusal, Unsupported};

/// Broad classification of a connection-opening failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectErrorKind {
    /// The server name could not be resolved.
    HostNotFound,
    /// The remote endpoint refused the connection.
    Refused,
    /// The remote endpoint could not be reached.
    Unreachable,
    /// The server certificate was rejected.
    CertificateRejected,
    /// TLS or WebSocket negotiation failed.
    HandshakeFailed,
    /// This build or target cannot open the requested endpoint.
    Unavailable,
    /// Another connection-opening failure occurred.
    Other,
}

/// An opaque, cloneable error from opening a network connection.
#[derive(Clone, Debug)]
pub struct ConnectError {
    kind: ConnectErrorKind,
    source: Arc<dyn std::error::Error + Send + Sync>,
}

impl ConnectError {
    /// Creates a connection error with a stable category and underlying cause.
    pub fn new(
        kind: ConnectErrorKind,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            kind,
            source: Arc::from(source.into()),
        }
    }

    /// Returns the stable category of this failure.
    #[must_use]
    pub fn kind(&self) -> ConnectErrorKind {
        self.kind
    }

    fn classify(error: &crate::transport::ConnectError) -> ConnectErrorKind {
        match error {
            crate::transport::ConnectError::Unavailable(_)
            | crate::transport::ConnectError::TlsUnavailable => ConnectErrorKind::Unavailable,
            crate::transport::ConnectError::Resolve { .. } => ConnectErrorKind::HostNotFound,
            crate::transport::ConnectError::Tcp(error)
                if error.kind() == io::ErrorKind::ConnectionRefused =>
            {
                ConnectErrorKind::Refused
            }
            crate::transport::ConnectError::Tcp(_) => ConnectErrorKind::Unreachable,
            #[cfg(tls)]
            crate::transport::ConnectError::Tls(rustls::Error::InvalidCertificate(_)) => {
                ConnectErrorKind::CertificateRejected
            }
            #[cfg(tls)]
            crate::transport::ConnectError::Tls(_) => ConnectErrorKind::HandshakeFailed,
            #[cfg(tls)]
            crate::transport::ConnectError::InvalidServerName(_)
            | crate::transport::ConnectError::InvalidTlsConfiguration(_) => ConnectErrorKind::Other,
            #[cfg(all(feature = "websocket", sockets))]
            crate::transport::ConnectError::WebSocket(_) => ConnectErrorKind::HandshakeFailed,
            #[cfg(browser)]
            crate::transport::ConnectError::BrowserWebSocket(_) => {
                ConnectErrorKind::HandshakeFailed
            }
        }
    }
}

impl From<crate::transport::ConnectError> for ConnectError {
    fn from(error: crate::transport::ConnectError) -> Self {
        let kind = Self::classify(&error);
        Self {
            kind,
            source: Arc::new(error),
        }
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            ConnectErrorKind::HostNotFound => "the server host could not be found",
            ConnectErrorKind::Refused => "the server refused the connection",
            ConnectErrorKind::Unreachable => "the server could not be reached",
            ConnectErrorKind::CertificateRejected => "the server certificate was rejected",
            ConnectErrorKind::HandshakeFailed => "the connection handshake failed",
            ConnectErrorKind::Unavailable => "the requested connection transport is unavailable",
            ConnectErrorKind::Other => "the connection could not be opened",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
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
    /// Creates a numeric refusal using the same classifier as received replies.
    #[must_use]
    pub fn numeric(code: u16, params: Vec<String>) -> Self {
        if let Some(numeric) = papo_proto::Numeric::new(code) {
            return Self::from_core(papo_core::ServerError::Numeric { numeric, params });
        }
        Self {
            kind: Refusal::Other,
            command: String::new(),
            code: code.to_string(),
            description: params.last().cloned().unwrap_or_default(),
            params,
        }
    }

    /// Creates a standard-reply refusal using the same classifier as received replies.
    #[must_use]
    pub fn standard(
        command: impl Into<String>,
        code: impl Into<String>,
        context: Vec<String>,
        description: impl Into<String>,
    ) -> Self {
        let command = command.into();
        let code = code.into();
        Self::from_core(papo_core::ServerError::Standard {
            command,
            code,
            context,
            description: description.into(),
        })
    }

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
        let kind = error.refusal();
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

/// A failure while registering an IRC connection.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum RegistrationError {
    /// Required SASL authentication failed.
    #[error("SASL authentication failed: {message}")]
    Sasl {
        /// The bounded diagnostic reported by the server.
        message: String,
    },
    /// The server requires authentication but no account credentials were configured.
    ///
    /// [`crate::State::isupport`] exposes the corresponding ISUPPORT indication when supplied.
    #[error("the server requires an authenticated account")]
    AccountRequired,
    /// The server rejected its connection password.
    #[error("the server password is incorrect")]
    PasswordIncorrect,
    /// The server banned this connection.
    #[error("the server banned this connection: {message}")]
    Banned {
        /// The bounded diagnostic reported by the server.
        message: String,
    },
    /// The server rejected registration for another reason.
    #[error("the server rejected the connection: {message}")]
    Rejected {
        /// The bounded diagnostic reported by the server.
        message: String,
    },
    /// Registration did not finish before its deadline.
    #[error("IRC registration timed out")]
    Timeout,
    /// A server requested consecutive STS upgrades without registering.
    #[error("repeated STS upgrade for {host} without registration")]
    StsUpgradeLoop {
        /// The hostname whose server repeated the upgrade.
        host: String,
    },
}

/// Why an established or pending IRC connection ended.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DisconnectCause {
    /// The transport reached EOF or otherwise vanished.
    ConnectionLost,
    /// Opening a replacement connection failed.
    Connect(ConnectErrorKind),
    /// Registration failed on a replacement connection.
    Registration(RegistrationError),
    /// The server sent `ERROR` with bounded explanatory text.
    Server {
        /// The server's bounded explanation.
        message: String,
    },
    /// The server did not answer a keepalive probe.
    PingTimeout,
    /// Incoming data violated a protocol or resource bound.
    Protocol {
        /// A bounded diagnostic suitable for display.
        message: String,
    },
    /// The event consumer remained full past its bounded overflow.
    SlowConsumer,
    /// The client deliberately ended the connection.
    Requested,
}

impl fmt::Display for DisconnectCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionLost => formatter.write_str("the connection was lost"),
            Self::Connect(kind) => write!(formatter, "a connection attempt failed: {kind:?}"),
            Self::Registration(error) => error.fmt(formatter),
            Self::Server { message } => write!(formatter, "the server disconnected: {message}"),
            Self::PingTimeout => formatter.write_str("the server stopped responding"),
            Self::Protocol { message } => write!(formatter, "protocol failure: {message}"),
            Self::SlowConsumer => formatter.write_str("the event consumer was too slow"),
            Self::Requested => formatter.write_str("the client disconnected"),
        }
    }
}

/// A disconnected operation and the driver's next retry delay.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Disconnected {
    /// Why the connection ended.
    pub cause: DisconnectCause,
    /// Delay before the next connection attempt, when one is scheduled.
    pub retry_in: Option<Duration>,
}

impl Disconnected {
    /// Creates a disconnected-operation description.
    #[must_use]
    pub const fn new(cause: DisconnectCause, retry_in: Option<Duration>) -> Self {
        Self { cause, retry_in }
    }

    /// Whether another connection attempt is scheduled.
    #[must_use]
    pub const fn is_retrying(&self) -> bool {
        self.retry_in.is_some()
    }
}

impl fmt::Display for Disconnected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for Disconnected {}

/// An error produced by the asynchronous IRC client.
#[derive(Clone, Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An input value is invalid.
    #[error(transparent)]
    InvalidInput(#[from] InvalidInput),
    /// Opening the connection failed.
    #[error(transparent)]
    Connect(#[from] ConnectError),
    /// The requested optional IRC behavior is unavailable.
    #[error(transparent)]
    Unsupported(Unsupported),
    /// The connection already has 256 requests awaiting replies.
    #[error("too many IRC requests are awaiting replies")]
    TooManyPendingRequests,
    /// Registration ended before the server welcomed the client.
    #[error(transparent)]
    Registration(RegistrationError),
    /// The established connection ended.
    #[error(transparent)]
    Disconnected(Disconnected),
    /// An operation did not complete before its deadline.
    #[error("the IRC operation timed out")]
    Timeout,
    /// The server's reply did not have the shape required by its command.
    #[error("the server sent an unexpected reply to {command}")]
    UnexpectedReply {
        /// The command whose reply was malformed or incomplete.
        command: String,
    },
    /// The IRC server rejected a request.
    #[error(transparent)]
    Server(ServerError),
}

impl Error {
    pub(crate) fn unexpected_reply(command: impl Into<String>) -> Self {
        Self::UnexpectedReply {
            command: command.into(),
        }
    }

    pub(crate) const fn disconnected() -> Self {
        Self::Disconnected(Disconnected::new(DisconnectCause::ConnectionLost, None))
    }

    pub(crate) fn disconnected_protocol(message: impl Into<String>) -> Self {
        Self::Disconnected(Disconnected::new(
            DisconnectCause::Protocol {
                message: message.into(),
            },
            None,
        ))
    }

    pub(crate) fn disconnected_close(kind: papo_core::CloseKind, message: String) -> Self {
        match kind {
            papo_core::CloseKind::SaslFailed => {
                Self::Registration(RegistrationError::Sasl { message })
            }
            papo_core::CloseKind::WebIrcRejected => {
                Self::Registration(RegistrationError::Rejected { message })
            }
            papo_core::CloseKind::RegistrationTimeout => {
                Self::Registration(RegistrationError::Timeout)
            }
            papo_core::CloseKind::AccountRequired => {
                Self::Registration(RegistrationError::AccountRequired)
            }
            papo_core::CloseKind::PasswordIncorrect => {
                Self::Registration(RegistrationError::PasswordIncorrect)
            }
            papo_core::CloseKind::Banned => {
                Self::Registration(RegistrationError::Banned { message })
            }
            papo_core::CloseKind::ServerError => {
                Self::Disconnected(Disconnected::new(DisconnectCause::Server { message }, None))
            }
            papo_core::CloseKind::PingTimeout => {
                Self::Disconnected(Disconnected::new(DisconnectCause::PingTimeout, None))
            }
            _ => Self::disconnected_protocol(message),
        }
    }

    pub(crate) const fn disconnected_slow_consumer() -> Self {
        Self::Disconnected(Disconnected::new(DisconnectCause::SlowConsumer, None))
    }

    pub(crate) const fn disconnected_requested() -> Self {
        Self::Disconnected(Disconnected::new(DisconnectCause::Requested, None))
    }

    #[allow(dead_code)]
    pub(crate) fn connect_unavailable(transport: &'static str) -> Self {
        Self::Connect(crate::transport::ConnectError::Unavailable(transport).into())
    }

    pub(crate) fn build(error: papo_proto::BuildError) -> Self {
        Self::InvalidInput(InvalidInput::InvalidMessage(error.to_string()))
    }

    pub(crate) fn server(error: papo_core::ServerError) -> Self {
        Self::Server(ServerError::from_core(error))
    }

    pub(crate) fn request(error: papo_core::RequestError) -> Self {
        use papo_core::RequestError as Request;
        match error {
            Request::Build(error) => {
                Self::InvalidInput(InvalidInput::InvalidMessage(error.to_string()))
            }
            Request::EmptyText => Self::InvalidInput(InvalidInput::EmptyText),
            Request::InvalidSecret => Self::InvalidInput(InvalidInput::BinarySecret),
            Request::PendingLimit => Self::TooManyPendingRequests,
            Request::Unsupported(unsupported) => Self::Unsupported(unsupported),
            Request::InvalidMessageReference => {
                Self::InvalidInput(InvalidInput::UnusableMessageReference)
            }
            Request::EmailRequired => Self::InvalidInput(InvalidInput::EmailRequired),
            Request::CustomAccountNameUnsupported => {
                Self::InvalidInput(InvalidInput::AccountNameMustMatchNick)
            }
            Request::InvalidMetadataKey { key, reason } => {
                Self::InvalidInput(InvalidInput::invalid_name(key, reason))
            }
            Request::MetadataValueTooLong { maximum, actual } => {
                Self::InvalidInput(InvalidInput::MetadataValueTooLong { maximum, actual })
            }
            Request::TooManyMetadataSubscriptions { maximum, actual } => {
                Self::InvalidInput(InvalidInput::TooManyMetadataSubscriptions { maximum, actual })
            }
            #[cfg(feature = "unstable")]
            Request::InvalidRelayNickname => Self::InvalidInput(InvalidInput::InvalidRelayNickname),
            #[cfg(feature = "unstable")]
            Request::InvalidWebPushEndpoint => {
                Self::InvalidInput(InvalidInput::InvalidWebPushEndpoint)
            }
            #[cfg(feature = "unstable")]
            Request::InvalidWebPushKey => Self::InvalidInput(InvalidInput::InvalidWebPushKey),
            _ => Self::InvalidInput(InvalidInput::InvalidMessage(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, time::Duration};

    use super::{
        ConnectError, ConnectErrorKind, DisconnectCause, Disconnected, Error, Refusal,
        RegistrationError, ServerError,
    };
    use crate::InvalidInput;

    #[test]
    fn terminal_payloads_preserve_bounded_diagnostics() {
        let registration = RegistrationError::Banned {
            message: "K-lined: open proxy".into(),
        };
        assert!(registration.to_string().contains("K-lined: open proxy"));
        let disconnected = Disconnected {
            cause: DisconnectCause::Server {
                message: "Closing Link: maintenance".into(),
            },
            retry_in: Some(Duration::from_secs(8)),
        };
        assert!(disconnected.to_string().contains("maintenance"));
        assert_eq!(disconnected.retry_in, Some(Duration::from_secs(8)));
    }

    #[test]
    fn server_errors_hide_core_shapes_behind_accessors() {
        let error = ServerError::numeric(
            404,
            vec!["papo-driver".into(), "#rust".into(), "Cannot send".into()],
        );
        assert_eq!(error.kind(), Refusal::CannotSendToChannel);
        assert_eq!(error.command(), "");
        assert_eq!(error.code(), "404");
        assert_eq!(error.description(), "Cannot send");
        assert_eq!(error.params()[1], "#rust");
        assert_eq!(error.to_string(), "server refusal 404: Cannot send");

        let standard = ServerError::standard(
            "CHATHISTORY",
            "INVALID_PARAMS",
            vec!["LATEST".into()],
            "Invalid parameters",
        );
        assert_eq!(standard.command(), "CHATHISTORY");
        assert_eq!(standard.code(), "INVALID_PARAMS");
        assert_eq!(standard.params(), ["LATEST"]);
    }

    #[cfg(feature = "unstable")]
    #[test]
    fn classifies_bouncer_network_failures() {
        for (code, expected) in [
            ("INVALID_NETID", Refusal::InvalidNetwork),
            ("INVALID_ATTRIBUTE", Refusal::InvalidAttribute),
            ("READ_ONLY_ATTRIBUTE", Refusal::ReadOnlyAttribute),
            ("UNKNOWN_ATTRIBUTE", Refusal::UnknownAttribute),
            ("NEED_ATTRIBUTE", Refusal::MissingAttribute),
        ] {
            let error = ServerError::from_core(papo_core::ServerError::Standard {
                command: "BOUNCER".into(),
                code: code.into(),
                context: vec!["CHANGENETWORK".into(), "42".into()],
                description: "Network request rejected".into(),
            });
            assert_eq!(error.kind(), expected);
        }
    }

    #[test]
    fn connection_error_exposes_only_its_stable_kind() {
        let error = ConnectError::from(crate::transport::ConnectError::Tcp(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "recorded refusal",
        )));
        assert_eq!(error.kind(), ConnectErrorKind::Refused);
        assert_eq!(error.to_string(), "the server refused the connection");
        assert!(std::error::Error::source(&error).is_some());
        assert_eq!(error.clone().kind(), ConnectErrorKind::Refused);

        let external = ConnectError::new(
            ConnectErrorKind::HostNotFound,
            io::Error::new(io::ErrorKind::NotFound, "missing.example"),
        );
        assert_eq!(external.kind(), ConnectErrorKind::HostNotFound);
        assert_eq!(external.to_string(), "the server host could not be found");
    }

    #[test]
    fn protocol_disconnect_keeps_its_diagnostic() {
        let error = Error::Disconnected(Disconnected::new(
            DisconnectCause::Protocol {
                message: "connection reset by peer".into(),
            },
            None,
        ));
        assert_eq!(
            error.to_string(),
            "protocol failure: connection reset by peer"
        );
    }

    #[test]
    fn terminal_errors_have_distinct_messages() {
        assert_eq!(
            Error::Disconnected(Disconnected::new(DisconnectCause::ConnectionLost, None))
                .to_string(),
            "the connection was lost"
        );
        assert_eq!(Error::Timeout.to_string(), "the IRC operation timed out");
        assert_eq!(
            Error::InvalidInput(InvalidInput::InvalidMessage("invalid IRC line".into()))
                .to_string(),
            "invalid IRC message: invalid IRC line"
        );
    }
}
