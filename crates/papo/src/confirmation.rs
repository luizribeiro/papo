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
        /// Network-unique identifier the server assigned.
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
}

pub(crate) fn echo_time(echo: &papo_core::SentInfo) -> Option<ServerTime> {
    echo.server_time
        .as_deref()
        .and_then(papo_proto::time::parse)
}
