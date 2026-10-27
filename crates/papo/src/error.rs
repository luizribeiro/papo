use std::{fmt, io, sync::Arc, time::Duration};

use thiserror::Error;

use crate::{InvalidInput, Refusal, Unsupported};

/// Broad classification of a connection or login failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ConnectErrorKind {
    /// The server name could not be resolved.
    HostNotFound,
    /// The remote endpoint refused the connection.
    Refused,
    /// The remote endpoint could not be reached.
    Unreachable,
    /// Opening the network or TLS connection timed out.
    TimedOut,
    /// The server certificate was rejected.
    CertificateRejected,
    /// TLS or WebSocket negotiation failed.
    HandshakeFailed,
    /// This build or target cannot open the requested endpoint.
    Unavailable,
    /// Required SASL authentication failed.
    Sasl,
    /// The server does not offer a usable SASL mechanism.
    SaslUnavailable,
    /// An IRC capability required by the connection settings is unavailable.
    CapabilityUnavailable,
    /// The server requires an authenticated account.
    AccountRequired,
    /// The server rejected the connection password.
    PasswordIncorrect,
    /// The server banned this connection.
    Banned,
    /// The server rejected the gateway's `WEBIRC` command.
    WebIrcRejected,
    /// A server requested consecutive STS upgrades without completing login.
    StsUpgradeLoop,
    /// The server rejected login for another reason.
    Server,
    /// The server did not welcome the client before the login deadline.
    WelcomeTimedOut,
    /// Another connection or login failure occurred.
    Other,
}

/// An opaque, cloneable error from opening a network connection or logging in.
///
/// Equality compares the stable [`ConnectErrorKind`] and the text of the
/// underlying error.
#[derive(Clone, Debug)]
pub struct ConnectError {
    kind: ConnectErrorKind,
    source: Arc<dyn std::error::Error + Send + Sync>,
}

impl PartialEq for ConnectError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.source.to_string() == other.source.to_string()
    }
}

impl Eq for ConnectError {}

impl ConnectError {
    pub(crate) fn new(
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

    pub(crate) fn is_configuration_failure(&self) -> bool {
        if self.kind == ConnectErrorKind::Unavailable {
            return true;
        }
        #[cfg(tls)]
        return self
            .source
            .downcast_ref::<crate::transport::ConnectError>()
            .is_some_and(|error| {
                matches!(
                    error,
                    crate::transport::ConnectError::InvalidServerName(_)
                        | crate::transport::ConnectError::InvalidTlsConfiguration(_)
                )
            });
        #[cfg(not(tls))]
        false
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
            crate::transport::ConnectError::Tcp(error)
                if error.kind() == io::ErrorKind::TimedOut =>
            {
                ConnectErrorKind::TimedOut
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
            | crate::transport::ConnectError::InvalidTlsConfiguration(_) => {
                ConnectErrorKind::HandshakeFailed
            }
            #[cfg(all(feature = "websocket", sockets))]
            crate::transport::ConnectError::WebSocket(_) => ConnectErrorKind::HandshakeFailed,
            #[cfg(browser)]
            crate::transport::ConnectError::BrowserWebSocket(_) => {
                ConnectErrorKind::HandshakeFailed
            }
        }
    }

    pub(crate) fn login(kind: ConnectErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        let message = if displays_server_reason(kind) {
            papo_proto::display_text(&message).into_owned()
        } else {
            message
        };
        Self::new(kind, ConnectFailure(message))
    }
}

const fn displays_server_reason(kind: ConnectErrorKind) -> bool {
    matches!(
        kind,
        ConnectErrorKind::Banned
            | ConnectErrorKind::Server
            | ConnectErrorKind::PasswordIncorrect
            | ConnectErrorKind::AccountRequired
            | ConnectErrorKind::WebIrcRejected
            | ConnectErrorKind::Sasl
    )
}

#[derive(Debug, Error)]
#[error("{0}")]
struct ConnectFailure(String);

impl From<crate::transport::ConnectError> for ConnectError {
    fn from(error: crate::transport::ConnectError) -> Self {
        let kind = Self::classify(&error);
        Self::new(kind, error)
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ConnectErrorKind::CapabilityUnavailable {
            return self.source.fmt(formatter);
        }
        let message = match self.kind {
            ConnectErrorKind::HostNotFound => "the server host could not be found",
            ConnectErrorKind::Refused => "the server refused the connection",
            ConnectErrorKind::Unreachable => "the server could not be reached",
            ConnectErrorKind::TimedOut => "the connection attempt timed out",
            ConnectErrorKind::CertificateRejected => "the server certificate was rejected",
            ConnectErrorKind::HandshakeFailed => "the connection handshake failed",
            ConnectErrorKind::Unavailable => "the requested connection transport is unavailable",
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

/// A server refusal with its wire-level diagnostic context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerError {
    refusal: Refusal,
    numeric: Option<u16>,
    command: String,
    code: String,
    description: String,
    params: Vec<String>,
}

impl ServerError {
    /// Creates a numeric refusal using the same classifier as received replies.
    #[must_use]
    pub fn from_numeric<I, S>(code: u16, params: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let params = params.into_iter().map(Into::into).collect::<Vec<_>>();
        if let Some(numeric) = papo_proto::Numeric::new(code) {
            return Self::from_core(papo_core::ServerError::__numeric(numeric, params));
        }
        Self {
            refusal: Refusal::Other,
            numeric: Some(code),
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
        description: impl Into<String>,
    ) -> Self {
        let command = command.into();
        let code = code.into();
        Self::from_core(papo_core::ServerError::__standard(
            command,
            code,
            Vec::new(),
            description.into(),
        ))
    }

    /// Supplies the standard reply's diagnostic parameters in wire order.
    #[must_use]
    pub fn context<I, S>(mut self, context: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.params = context.into_iter().map(Into::into).collect();
        self
    }

    /// Returns the stable refusal category.
    #[must_use]
    pub const fn refusal(&self) -> Refusal {
        self.refusal
    }

    /// Returns the three-digit reply number, or `None` for a standard reply.
    #[must_use]
    pub const fn numeric(&self) -> Option<u16> {
        self.numeric
    }

    /// Returns the command named by a standard reply, or `None` for a numeric.
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        self.numeric.is_none().then_some(&self.command)
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
                ..
            } => Self {
                refusal: kind,
                numeric: None,
                command,
                code,
                description,
                params: context,
            },
            papo_core::ServerError::Numeric {
                numeric, params, ..
            } => Self {
                refusal: kind,
                numeric: Some(numeric.code()),
                command: String::new(),
                code: format!("{:03}", numeric.code()),
                description: params.last().cloned().unwrap_or_default(),
                params,
            },
            other => Self {
                refusal: Refusal::Other,
                numeric: None,
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
        let description = papo_proto::display_text(&self.description);
        if self.command.is_empty() {
            write!(formatter, "server refusal {}: {description}", self.code)
        } else {
            write!(formatter, "{} {}: {description}", self.command, self.code)
        }
    }
}

impl std::error::Error for ServerError {}

/// Why an established or pending IRC connection ended.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DisconnectCause {
    /// The transport reached EOF or otherwise vanished.
    #[non_exhaustive]
    ConnectionLost {
        /// A bounded transport diagnostic.
        message: String,
    },
    /// Opening a replacement connection or completing its login failed.
    Connect(ConnectError),
    /// The server sent `ERROR` with bounded explanatory text.
    #[non_exhaustive]
    Server {
        /// The server's bounded explanation.
        message: String,
    },
    /// The server did not answer a keepalive probe.
    PingTimeout,
    /// Incoming data violated a protocol or resource bound.
    #[non_exhaustive]
    Protocol {
        /// A bounded diagnostic suitable for display.
        message: String,
    },
    /// The event consumer remained full past its bounded overflow.
    ///
    /// The disconnect follows the configured reconnect policy. Move slow work
    /// out of the event loop or raise [`Builder::event_capacity`](crate::Builder::event_capacity)
    /// to avoid it.
    SlowConsumer,
    /// The client deliberately ended the connection.
    #[non_exhaustive]
    Requested {
        /// Whether the peer failed to acknowledge `QUIT` within ten seconds.
        quit_unacknowledged: bool,
    },
}

impl DisconnectCause {
    pub(crate) fn connection_lost(message: impl Into<String>) -> Self {
        let message = message.into();
        let end = message.floor_char_boundary(message.len().min(256));
        Self::ConnectionLost {
            message: message[..end].to_owned(),
        }
    }
}

impl fmt::Display for DisconnectCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionLost { message } => {
                write!(
                    formatter,
                    "the connection was lost: {}",
                    papo_proto::display_text(message)
                )
            }
            Self::Connect(error) => error.fmt(formatter),
            Self::Server { message } => write!(
                formatter,
                "the server disconnected: {}",
                papo_proto::display_text(message)
            ),
            Self::PingTimeout => formatter.write_str("the server stopped responding"),
            Self::Protocol { message } => write!(
                formatter,
                "protocol failure: {}",
                papo_proto::display_text(message)
            ),
            Self::SlowConsumer => formatter.write_str("the event consumer was too slow"),
            Self::Requested { .. } => formatter.write_str("the client disconnected"),
        }
    }
}

/// Details of a connection that ended.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Disconnected {
    /// Why the connection ended.
    pub cause: DisconnectCause,
    /// Delay before the next connection attempt.
    ///
    /// `None` means papo stopped because the cause was fatal, reconnecting was disabled or
    /// exhausted, or the client requested a quit.
    pub reconnect_in: Option<Duration>,
}

impl Disconnected {
    /// Creates a disconnected-operation description.
    #[must_use]
    pub const fn new(cause: DisconnectCause, reconnect_in: Option<Duration>) -> Self {
        Self {
            cause,
            reconnect_in,
        }
    }

    /// Whether another connection attempt is scheduled.
    #[must_use]
    pub const fn is_reconnecting(&self) -> bool {
        self.reconnect_in.is_some()
    }
}

impl fmt::Display for Disconnected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

impl std::error::Error for Disconnected {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.cause)
    }
}

impl std::error::Error for DisconnectCause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) => std::error::Error::source(error),
            _ => None,
        }
    }
}

/// An error produced by the asynchronous IRC client.
#[derive(Clone, Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An input value is invalid.
    #[error(transparent)]
    InvalidInput(#[from] InvalidInput),
    /// Opening the connection or completing its login failed.
    #[error(transparent)]
    Connect(#[from] ConnectError),
    /// The requested optional IRC behavior is unavailable.
    #[error(transparent)]
    Unsupported(Unsupported),
    /// The connection already has 256 requests awaiting replies.
    #[error("too many IRC requests are awaiting replies")]
    TooManyPendingRequests,
    /// The operation was not written because the connection ended.
    #[error(transparent)]
    Disconnected(Disconnected),
    /// The connection ended after transmission began but before the reply arrived.
    ///
    /// The server may already have acted, so retrying a non-idempotent operation
    /// can repeat its effect.
    #[error("the IRC operation was interrupted: {0}")]
    Interrupted(Disconnected),
    /// Every line was sent, but the expected reply did not arrive in time.
    ///
    /// The server may already have acted. Retrying an action such as `KICK` or
    /// `MODE +b` can repeat its effect.
    #[error("the IRC operation timed out")]
    Timeout,
    /// Server input exceeded a bounded state collection.
    #[error("server input exceeded the {0:?} state limit")]
    StateLimit(crate::StateLimit),
    /// A requested state change could not be confirmed from server replies or state.
    #[error("the IRC operation could not be confirmed")]
    Unconfirmed,
    /// The intended user left, changed nickname, or no longer holds a private reply nickname.
    #[error("the target user is no longer present")]
    TargetGone,
    /// The server's reply did not have the shape required by its command.
    #[error("the server sent an unexpected reply to {command}")]
    #[non_exhaustive]
    UnexpectedReply {
        /// The command whose reply was malformed or incomplete.
        command: String,
    },
    /// The IRC server rejected a request.
    #[error(transparent)]
    Server(ServerError),
}

impl From<ServerError> for Error {
    fn from(error: ServerError) -> Self {
        Self::Server(error)
    }
}

impl From<Unsupported> for Error {
    fn from(error: Unsupported) -> Self {
        Self::Unsupported(error)
    }
}

impl From<crate::StateLimit> for Error {
    fn from(error: crate::StateLimit) -> Self {
        Self::StateLimit(error)
    }
}

impl Error {
    pub(crate) fn transport(error: crate::transport::ConnectError) -> Self {
        #[cfg(tls)]
        if matches!(
            &error,
            crate::transport::ConnectError::InvalidServerName(_)
                | crate::transport::ConnectError::InvalidTlsConfiguration(_)
        ) {
            return Self::InvalidInput(InvalidInput::TlsConfig(error.to_string()));
        }
        Self::Connect(error.into())
    }

    pub(crate) fn unexpected_reply(command: impl Into<String>) -> Self {
        Self::UnexpectedReply {
            command: command.into(),
        }
    }

    pub(crate) const fn disconnected(disconnected: Disconnected) -> Self {
        Self::Disconnected(disconnected)
    }

    pub(crate) fn connection_lost() -> Self {
        Self::connection_lost_with("transport ended unexpectedly")
    }

    pub(crate) fn connection_lost_with(message: impl Into<String>) -> Self {
        Self::disconnected(Disconnected::new(
            DisconnectCause::connection_lost(message),
            None,
        ))
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
        let connect_kind = match kind {
            papo_core::CloseKind::SaslFailed => Some(ConnectErrorKind::Sasl),
            papo_core::CloseKind::SaslUnavailable => Some(ConnectErrorKind::SaslUnavailable),
            papo_core::CloseKind::CapabilityUnavailable => {
                Some(ConnectErrorKind::CapabilityUnavailable)
            }
            papo_core::CloseKind::WebIrcRejected => Some(ConnectErrorKind::WebIrcRejected),
            papo_core::CloseKind::WelcomeTimeout => Some(ConnectErrorKind::WelcomeTimedOut),
            papo_core::CloseKind::AccountRequired => Some(ConnectErrorKind::AccountRequired),
            papo_core::CloseKind::PasswordIncorrect => Some(ConnectErrorKind::PasswordIncorrect),
            papo_core::CloseKind::Banned => Some(ConnectErrorKind::Banned),
            papo_core::CloseKind::ConnectionRestricted
            | papo_core::CloseKind::NickCandidatesExhausted => Some(ConnectErrorKind::Server),
            _ => None,
        };
        if let Some(kind) = connect_kind {
            return Self::Connect(ConnectError::login(kind, message));
        }
        match kind {
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
        Self::Disconnected(Disconnected::new(
            DisconnectCause::Requested {
                quit_unacknowledged: false,
            },
            None,
        ))
    }

    #[allow(dead_code)]
    pub(crate) fn connect_unavailable(transport: &'static str) -> Self {
        Self::Connect(crate::transport::ConnectError::Unavailable(transport).into())
    }

    pub(crate) fn build(error: papo_proto::BuildError) -> Self {
        Self::InvalidInput(InvalidInput::WireMessage(error))
    }

    pub(crate) fn server(error: papo_core::ServerError) -> Self {
        Self::Server(ServerError::from_core(error))
    }

    pub(crate) fn request(error: papo_core::RequestError) -> Self {
        use papo_core::RequestError as Request;
        match error {
            Request::Build(error) => Self::InvalidInput(InvalidInput::WireMessage(error)),
            Request::EmptyText => Self::InvalidInput(InvalidInput::EmptyText),
            Request::InsecureAuthentication => {
                Self::InvalidInput(InvalidInput::InsecureAuthentication)
            }
            Request::PendingLimit => Self::TooManyPendingRequests,
            Request::Unsupported(unsupported) => Self::Unsupported(unsupported),
            Request::InvalidMessageReference => {
                Self::InvalidInput(InvalidInput::UnusableMessageReference)
            }
            Request::EmailRequired => Self::InvalidInput(InvalidInput::EmailRequired),
            Request::CustomAccountNameUnsupported => {
                Self::InvalidInput(InvalidInput::AccountNameMustMatchNick)
            }
            Request::InvalidMetadataKey { key, reason, .. } => {
                Self::InvalidInput(InvalidInput::InvalidMetadataKey { key, reason })
            }
            Request::MetadataValueTooLong {
                maximum, actual, ..
            } => Self::InvalidInput(InvalidInput::MetadataValueTooLong { maximum, actual }),
            Request::TooManyMetadataSubscriptions {
                maximum, actual, ..
            } => Self::InvalidInput(InvalidInput::TooManyMetadataSubscriptions { maximum, actual }),
            #[cfg(feature = "unstable")]
            Request::InvalidRelayNickname => Self::InvalidInput(InvalidInput::InvalidRelayNickname),
            #[cfg(feature = "unstable")]
            Request::InvalidWebPushEndpoint => {
                Self::InvalidInput(InvalidInput::InvalidWebPushEndpoint)
            }
            #[cfg(feature = "unstable")]
            Request::InvalidWebPushKey => Self::InvalidInput(InvalidInput::InvalidWebPushKey),
            _ => Self::InvalidInput(InvalidInput::InvalidRequest(error.to_string())),
        }
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{io, time::Duration};

    use super::{
        ConnectError, ConnectErrorKind, DisconnectCause, Disconnected, Error, Refusal, ServerError,
    };
    use crate::{Feature, InvalidInput, StateLimit, Unsupported, UnsupportedReason};

    #[test]
    fn terminal_payloads_preserve_bounded_diagnostics() {
        let disconnected = Disconnected {
            cause: DisconnectCause::Server {
                message: "Closing Link: maintenance".into(),
            },
            reconnect_in: Some(Duration::from_secs(8)),
        };
        assert!(disconnected.to_string().contains("maintenance"));
        assert_eq!(disconnected.reconnect_in, Some(Duration::from_secs(8)));
    }

    #[test]
    fn kickban_terminal_errors_are_distinct() {
        assert_eq!(
            Error::Unconfirmed.to_string(),
            "the IRC operation could not be confirmed"
        );
        assert_eq!(
            Error::TargetGone.to_string(),
            "the target user is no longer present"
        );
    }

    #[test]
    fn public_error_payloads_convert_to_their_matching_variants() {
        let server = ServerError::from_numeric(404, ["papo-driver", "#rust", "Cannot send"]);
        assert!(matches!(
            Error::from(server),
            Error::Server(error) if error.refusal() == Refusal::CannotSendToChannel
        ));

        let unsupported =
            Unsupported::new(Feature::MessageTags, UnsupportedReason::DisabledByClient);
        assert!(matches!(
            Error::from(unsupported),
            Error::Unsupported(error) if error == unsupported
        ));
        assert!(matches!(
            Error::from(StateLimit::MetadataKeys),
            Error::StateLimit(StateLimit::MetadataKeys)
        ));
    }

    #[test]
    fn server_errors_hide_core_shapes_behind_accessors() {
        let error = ServerError::from_numeric(404, ["papo-driver", "#rust", "Cannot send"]);
        assert_eq!(error.refusal(), Refusal::CannotSendToChannel);
        assert_eq!(error.numeric(), Some(404));
        assert_eq!(error.command(), None);
        assert_eq!(error.code(), "404");
        assert_eq!(error.description(), "Cannot send");
        assert_eq!(error.params()[1], "#rust");
        assert_eq!(error.to_string(), "server refusal 404: Cannot send");

        let standard = ServerError::standard("CHATHISTORY", "INVALID_PARAMS", "Invalid parameters")
            .context(["LATEST"]);
        assert_eq!(standard.command(), Some("CHATHISTORY"));
        assert_eq!(standard.refusal(), Refusal::InvalidParameters);
        assert_eq!(standard.numeric(), None);
        assert_eq!(standard.code(), "INVALID_PARAMS");
        assert_eq!(standard.params(), ["LATEST"]);
    }

    #[cfg(feature = "unstable")]
    #[test]
    fn unscoped_bouncer_network_failures_remain_other() {
        for code in [
            "INVALID_NETID",
            "INVALID_ATTRIBUTE",
            "READ_ONLY_ATTRIBUTE",
            "UNKNOWN_ATTRIBUTE",
            "NEED_ATTRIBUTE",
        ] {
            let error = ServerError::from_core(papo_core::ServerError::__standard(
                "BOUNCER".into(),
                code.into(),
                vec!["CHANGENETWORK".into(), "42".into()],
                "Network request rejected".into(),
            ));
            assert_eq!(error.refusal(), Refusal::Other);
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

        let unreachable = ConnectError::from(crate::transport::ConnectError::Tcp(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "peer reset",
        )));
        assert_eq!(unreachable.kind(), ConnectErrorKind::Unreachable);

        let timed_out = ConnectError::from(crate::transport::ConnectError::Tcp(io::Error::new(
            io::ErrorKind::TimedOut,
            "connection attempt expired",
        )));
        assert_eq!(timed_out.kind(), ConnectErrorKind::TimedOut);
    }

    #[test]
    fn hostile_terminal_numeric_reasons_strip_control_strings() {
        for (numeric, kind, reason, expected) in [
            (
                465,
                ConnectErrorKind::Banned,
                "\x1b]8;;https://malicious.example/ban\x07K-lined: open proxy\x1b]8;;\x07",
                "the server banned this connection: K-lined: open proxy",
            ),
            (
                904,
                ConnectErrorKind::Sasl,
                "\x1b]52;c;Y29ycmVjdCBob3JzZSBiYXR0ZXJ5IHN0YXBsZQ==\x1b\\SASL credentials rejected",
                "SASL authentication failed: SASL credentials rejected",
            ),
            (
                465,
                ConnectErrorKind::Banned,
                "\x1bP1;2;3|terminal payload\x1b\\K-lined: compromised host",
                "the server banned this connection: K-lined: compromised host",
            ),
            (
                465,
                ConnectErrorKind::Banned,
                "\x1b_private command\x1b\\\x1b^privacy message\x1b\\\x1bXsos message\x1b\\K-lined: abusive client",
                "the server banned this connection: K-lined: abusive client",
            ),
            (
                904,
                ConnectErrorKind::Sasl,
                "\u{0090}device control\u{009c}\u{009d}clipboard\u{0007}\u{009e}privacy\u{009c}\u{009f}application\u{009c}Account suspended",
                "SASL authentication failed: Account suspended",
            ),
            (
                904,
                ConnectErrorKind::Sasl,
                "SASL authentication failed\x1b]unterminated clipboard payload",
                "SASL authentication failed: SASL authentication failed",
            ),
        ] {
            let error = ConnectError::login(kind, reason);
            assert_eq!(error.to_string(), expected, "numeric {numeric}");
        }
    }

    #[test]
    fn server_and_disconnect_diagnostics_sanitize_display_but_keep_raw_text() {
        let payload = "\x1b]52;c;cm0gLXJmIH4=\x07\x1b[2JClosing Link\rforged";
        let standard = ServerError::standard("JOIN", "CHANNEL_FULL", payload);
        assert_eq!(standard.description(), payload);
        assert_eq!(
            standard.to_string(),
            "JOIN CHANNEL_FULL: Closing Linkforged"
        );

        let numeric = ServerError::from_numeric(404, ["papo", "#papo", payload]);
        assert_eq!(numeric.description(), payload);
        assert_eq!(
            numeric.to_string(),
            "server refusal 404: Closing Linkforged"
        );

        for (cause, expected) in [
            (
                DisconnectCause::ConnectionLost {
                    message: payload.into(),
                },
                "the connection was lost: Closing Linkforged",
            ),
            (
                DisconnectCause::Server {
                    message: payload.into(),
                },
                "the server disconnected: Closing Linkforged",
            ),
            (
                DisconnectCause::Protocol {
                    message: payload.into(),
                },
                "protocol failure: Closing Linkforged",
            ),
        ] {
            assert_eq!(cause.to_string(), expected);
            match cause {
                DisconnectCause::ConnectionLost { message }
                | DisconnectCause::Server { message }
                | DisconnectCause::Protocol { message } => assert_eq!(message, payload),
                _ => unreachable!(),
            }
        }
    }

    #[cfg(tls)]
    #[test]
    fn late_tls_configuration_errors_do_not_become_other() {
        let classified =
            ConnectError::from(crate::transport::ConnectError::InvalidTlsConfiguration(
                "invalid certificate PEM".into(),
            ));
        assert_eq!(classified.kind(), ConnectErrorKind::HandshakeFailed);

        assert!(matches!(
            Error::transport(crate::transport::ConnectError::InvalidServerName(
                "not a server name".into()
            )),
            Error::InvalidInput(InvalidInput::TlsConfig(message))
                if message.contains("invalid TLS server name")
        ));
    }

    #[test]
    fn connection_error_equality_uses_kind_and_source_text() {
        let refused = ConnectError::new(
            ConnectErrorKind::Refused,
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "irc.example.net:6697 refused the connection",
            ),
        );
        let same = ConnectError::new(
            ConnectErrorKind::Refused,
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "irc.example.net:6697 refused the connection",
            ),
        );
        let unreachable = ConnectError::new(
            ConnectErrorKind::Unreachable,
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "irc.example.net:6697 refused the connection",
            ),
        );
        let other_endpoint = ConnectError::new(
            ConnectErrorKind::Refused,
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "irc.backup.example.net:6697 refused the connection",
            ),
        );

        assert_eq!(refused, same);
        assert_ne!(refused, unreachable);
        assert_ne!(refused, other_endpoint);
    }

    #[test]
    fn disconnect_chains_never_repeat_the_parent_message() {
        fn sources(error: &(dyn std::error::Error + 'static)) -> Vec<String> {
            std::iter::successors(error.source(), |source| source.source())
                .map(ToString::to_string)
                .collect()
        }

        fn assert_no_repetition(error: &(dyn std::error::Error + 'static)) {
            let mut parent = error;
            while let Some(source) = parent.source() {
                assert_ne!(parent.to_string(), source.to_string());
                parent = source;
            }
        }

        for error in [
            io::Error::new(io::ErrorKind::ConnectionReset, "connection reset by peer"),
            io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "WebSocket closed with code 1001: server restart",
            ),
        ] {
            let diagnostic = error.to_string();
            let disconnected =
                Disconnected::new(DisconnectCause::connection_lost(diagnostic.clone()), None);
            assert_no_repetition(&disconnected);
            assert_eq!(sources(&disconnected), Vec::<String>::new());
        }

        let cause = DisconnectCause::Connect(ConnectError::new(
            ConnectErrorKind::Refused,
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "irc.example.net:6697 refused the connection",
            ),
        ));
        assert_eq!(
            sources(&cause),
            ["irc.example.net:6697 refused the connection"]
        );
        assert_no_repetition(&cause);

        let disconnected = Disconnected::new(cause, None);
        assert_eq!(
            sources(&disconnected),
            ["irc.example.net:6697 refused the connection"]
        );
        assert_no_repetition(&disconnected);
    }

    #[test]
    fn close_kinds_retain_their_specific_categories() {
        assert!(matches!(
            Error::disconnected_close(
                papo_core::CloseKind::WelcomeTimeout,
                "IRC welcome timed out before 001".into()
            ),
            Error::Connect(error) if error.kind() == ConnectErrorKind::WelcomeTimedOut
        ));
        assert!(matches!(
            Error::disconnected_close(papo_core::CloseKind::Banned, "K-lined: open proxy".into()),
            Error::Connect(error)
                if error.kind() == ConnectErrorKind::Banned
                    && error.to_string()
                        == "the server banned this connection: K-lined: open proxy"
                    && std::error::Error::source(&error).is_none()
        ));
        assert!(matches!(
            Error::disconnected_close(papo_core::CloseKind::PingTimeout, "ping timeout".into()),
            Error::Disconnected(Disconnected {
                cause: DisconnectCause::PingTimeout,
                reconnect_in: None,
            })
        ));
    }

    #[test]
    fn request_errors_retain_their_public_categories() {
        use papo_core::RequestError as Request;

        assert!(matches!(
            Error::request(Request::PendingLimit),
            Error::TooManyPendingRequests
        ));
        assert!(matches!(
            Error::request(Request::Build(papo_proto::BuildError::InvalidParam(1))),
            Error::InvalidInput(InvalidInput::WireMessage(
                papo_proto::BuildError::InvalidParam(1)
            ))
        ));
        assert!(matches!(
            Error::request(Request::InvalidMessageReference),
            Error::InvalidInput(InvalidInput::UnusableMessageReference)
        ));
        assert!(matches!(
            Error::request(Request::EmailRequired),
            Error::InvalidInput(InvalidInput::EmailRequired)
        ));
        assert!(matches!(
            Error::request(Request::CustomAccountNameUnsupported),
            Error::InvalidInput(InvalidInput::AccountNameMustMatchNick)
        ));
        assert!(matches!(
            Error::request(Request::__invalid_metadata_key(
                "draft/example".into(),
                papo_proto::InvalidIdentifier::InvalidCharacter('/'),
            )),
            Error::InvalidInput(InvalidInput::InvalidMetadataKey { key, reason, .. })
                if key == "draft/example"
                    && reason == papo_proto::InvalidIdentifier::InvalidCharacter('/')
        ));
        assert!(matches!(
            Error::request(Request::__metadata_value_too_long(512, 513)),
            Error::InvalidInput(InvalidInput::MetadataValueTooLong {
                maximum: 512,
                actual: 513,
            })
        ));
        assert!(matches!(
            Error::request(Request::__too_many_metadata_subscriptions(10, 11)),
            Error::InvalidInput(InvalidInput::TooManyMetadataSubscriptions {
                maximum: 10,
                actual: 11,
            })
        ));

        #[cfg(feature = "unstable")]
        for (request, expected) in [
            (
                Request::InvalidRelayNickname,
                InvalidInput::InvalidRelayNickname,
            ),
            (
                Request::InvalidWebPushEndpoint,
                InvalidInput::InvalidWebPushEndpoint,
            ),
            (Request::InvalidWebPushKey, InvalidInput::InvalidWebPushKey),
        ] {
            assert!(matches!(
                Error::request(request),
                Error::InvalidInput(actual) if actual == expected
            ));
        }
    }

    #[test]
    fn every_public_error_leaf_can_be_constructed() {
        for kind in [
            ConnectErrorKind::HostNotFound,
            ConnectErrorKind::Refused,
            ConnectErrorKind::Unreachable,
            ConnectErrorKind::TimedOut,
            ConnectErrorKind::CertificateRejected,
            ConnectErrorKind::HandshakeFailed,
            ConnectErrorKind::Unavailable,
            ConnectErrorKind::Sasl,
            ConnectErrorKind::SaslUnavailable,
            ConnectErrorKind::CapabilityUnavailable,
            ConnectErrorKind::AccountRequired,
            ConnectErrorKind::PasswordIncorrect,
            ConnectErrorKind::Banned,
            ConnectErrorKind::WebIrcRejected,
            ConnectErrorKind::StsUpgradeLoop,
            ConnectErrorKind::Server,
            ConnectErrorKind::WelcomeTimedOut,
            ConnectErrorKind::Other,
        ] {
            let error = ConnectError::new(kind, io::Error::other("recorded cause"));
            assert_eq!(error.kind(), kind);
            assert_eq!(
                std::error::Error::source(&error).is_none(),
                super::displays_server_reason(kind)
            );
        }

        for cause in [
            DisconnectCause::ConnectionLost {
                message: "connection reset by peer".into(),
            },
            DisconnectCause::Connect(ConnectError::new(
                ConnectErrorKind::Refused,
                io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused"),
            )),
            DisconnectCause::Server {
                message: "maintenance".into(),
            },
            DisconnectCause::PingTimeout,
            DisconnectCause::Protocol {
                message: "overlong line".into(),
            },
            DisconnectCause::SlowConsumer,
            DisconnectCause::Requested {
                quit_unacknowledged: false,
            },
        ] {
            assert!(!cause.to_string().is_empty());
        }
    }

    #[test]
    fn every_public_invalid_input_variant_can_be_constructed() {
        let variants = [
            InvalidInput::Url(crate::InvalidUrl::Empty),
            InvalidInput::TlsConfig("invalid PEM".into()),
            InvalidInput::KnownCapability {
                capability: "chathistory".into(),
                feature: "History",
            },
            InvalidInput::UnstableCapability("draft/persistence".into()),
            InvalidInput::ManagedCapability("sts".into()),
            InvalidInput::ZeroEventCapacity,
            InvalidInput::RateLimit(crate::InvalidRateLimit::EmptyBurst),
            InvalidInput::WireMessage(crate::BuildError::InvalidParam(0)),
            InvalidInput::InvalidCtcp,
            InvalidInput::InvalidClientTagKey("account".into()),
            InvalidInput::CustomStreamNotReusable,
            InvalidInput::InvalidMode("mode has an invalid argument".into()),
            InvalidInput::InvalidMetadataKey {
                key: "Display-Name".into(),
                reason: crate::InvalidIdentifier::InvalidCharacter('D'),
            },
            InvalidInput::InvalidRequest("unsupported history status".into()),
            InvalidInput::InvalidName {
                argument: "name",
                name: String::new(),
                reason: crate::InvalidIdentifier::Empty,
            },
            InvalidInput::EmptyText,
            InvalidInput::Reaction {
                maximum: 4094,
                actual: 4095,
            },
            InvalidInput::UnusableMessageReference,
            InvalidInput::ForeignReference,
            InvalidInput::EmailRequired,
            InvalidInput::AccountNameMustMatchNick,
            InvalidInput::MetadataValueTooLong {
                maximum: 512,
                actual: 513,
            },
            InvalidInput::TooManyMetadataSubscriptions {
                maximum: 10,
                actual: 11,
            },
        ];
        assert!(variants.iter().all(|error| !error.to_string().is_empty()));

        #[cfg(feature = "unstable")]
        for error in [
            InvalidInput::InvalidRelayNickname,
            InvalidInput::InvalidWebPushEndpoint,
            InvalidInput::InvalidWebPushKey,
        ] {
            assert!(!error.to_string().is_empty());
        }
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
    fn connection_loss_diagnostics_stop_at_the_bound() {
        for input in ["x".repeat(256), "x".repeat(257)] {
            let Error::Disconnected(Disconnected {
                cause: DisconnectCause::ConnectionLost { message },
                ..
            }) = Error::connection_lost_with(input)
            else {
                panic!("expected connection loss")
            };
            assert_eq!(message.len(), 256);
        }
    }

    #[test]
    fn terminal_errors_have_distinct_messages() {
        assert_eq!(
            Error::Disconnected(Disconnected::new(
                DisconnectCause::ConnectionLost {
                    message: "connection reset by peer".into(),
                },
                None,
            ))
            .to_string(),
            "the connection was lost: connection reset by peer"
        );
        assert_eq!(Error::Timeout.to_string(), "the IRC operation timed out");
        assert_eq!(
            Error::Interrupted(Disconnected::new(
                DisconnectCause::Server {
                    message: "maintenance restart".into(),
                },
                None,
            ))
            .to_string(),
            "the IRC operation was interrupted: the server disconnected: maintenance restart"
        );
        assert_eq!(
            Error::InvalidInput(InvalidInput::WireMessage(crate::BuildError::InvalidCommand))
                .to_string(),
            "invalid IRC message: IRC command is invalid"
        );
    }
}
