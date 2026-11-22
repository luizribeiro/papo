use crate::{MessageId, ServerTime};

/// Whether the server confirmed a write by echoing it with the identity it assigned.
///
/// A refusal or a timeout is an error of the request, never [`Confirmation::Unconfirmed`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Confirmation {
    /// The server echoed the write with the identifier it assigned.
    #[non_exhaustive]
    Confirmed {
        /// The [`MessageId`] the server assigned to the echoed write.
        msgid: MessageId,
        /// Server time of the echo, when the server supplied one.
        server_time: Option<ServerTime>,
    },
    /// This server cannot confirm the write.
    #[non_exhaustive]
    Unconfirmed {
        /// What the server lacks to confirm it.
        reason: UnconfirmedReason,
    },
}

/// Why a server cannot confirm a write.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum UnconfirmedReason {
    /// `echo-message` is not enabled, so the server does not echo our writes.
    NoEchoMessage,
    /// `labeled-response` is not enabled, so an echo cannot be tied to its request.
    ///
    /// A message queued before the connection registered is also written without a label.
    NoLabeledResponse,
    /// The echo arrived without a `msgid` tag.
    NoMessageId,
}

impl Confirmation {
    pub(crate) fn unconfirmed(reason: UnconfirmedReason) -> Self {
        Self::Unconfirmed { reason }
    }

    pub(crate) fn from_labelled_echo(
        echo_message: bool,
        echo: Option<papo_core::SentInfo>,
    ) -> Self {
        match echo {
            Some(echo) => Self::Confirmed {
                server_time: echo_time(&echo),
                msgid: MessageId::from(echo.msgid),
            },
            None if echo_message => Self::unconfirmed(UnconfirmedReason::NoMessageId),
            None => Self::unconfirmed(UnconfirmedReason::NoEchoMessage),
        }
    }

    /// Decides one `REDACT`'s confirmation from the echo core tied to its response.
    ///
    /// With `echo-message` the server owes us that echo, so an answer without it has not proven
    /// the redaction.
    pub(crate) fn from_redaction(
        echo_message: bool,
        echo: Option<&papo_proto::LineRef<'_>>,
    ) -> Result<Self, crate::Error> {
        match echo {
            Some(echo) => Ok(Self::from_echo(echo)),
            None if echo_message => Err(crate::Error::NotVerified),
            None => Ok(Self::unconfirmed(UnconfirmedReason::NoEchoMessage)),
        }
    }

    pub(crate) fn from_echo(echo: &papo_proto::LineRef<'_>) -> Self {
        match echo.tag("msgid") {
            Some(msgid) => Self::Confirmed {
                msgid: MessageId::from(msgid.into_owned()),
                server_time: echo
                    .tag("time")
                    .as_deref()
                    .and_then(|value| ServerTime::from_stored(value).ok()),
            },
            None => Self::unconfirmed(UnconfirmedReason::NoMessageId),
        }
    }
}

pub(crate) fn echo_time(echo: &papo_core::SentInfo) -> Option<ServerTime> {
    echo.server_time
        .as_deref()
        .and_then(|value| ServerTime::from_stored(value).ok())
}
