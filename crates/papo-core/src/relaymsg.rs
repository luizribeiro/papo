use web_time::Instant;

use papo_proto::Message;

use crate::{Feature, RequestError, RequestId, Session};

/// A typed `FAIL RELAYMSG` code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RelayError {
    /// Relay messaging is disabled by server configuration.
    NotEnabled,
    /// The client lacks permission to relay a message.
    PrivilegesNeeded,
    /// The supplied message text is blank.
    BlankMessage,
    /// The relayed nickname is invalid.
    InvalidNick,
    /// The relayed sender is banned from the channel.
    Banned,
}

impl RelayError {
    pub(crate) fn parse(command: &str, code: &str) -> Option<Self> {
        if !command.eq_ignore_ascii_case("RELAYMSG") {
            return None;
        }
        match code.to_ascii_uppercase().as_str() {
            "NOT_ENABLED" => Some(Self::NotEnabled),
            "PRIVS_NEEDED" => Some(Self::PrivilegesNeeded),
            "BLANK_MSG" => Some(Self::BlankMessage),
            "INVALID_NICK" => Some(Self::InvalidNick),
            "BANNED" => Some(Self::Banned),
            _ => None,
        }
    }
}

impl Session {
    /// Sends a message attributed to a nickname on an external relay.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when `draft/relaymsg` was not negotiated,
    /// [`RequestError::InvalidRelayNickname`] when the nickname has no advertised separator,
    /// or another request-queueing error from [`Session::request`].
    pub fn relay(
        &mut self,
        now: Instant,
        channel: &str,
        nick_with_separator: &str,
        text: &str,
    ) -> Result<RequestId, RequestError> {
        self.require_feature(Feature::Relay)?;
        let separators = self
            .caps
            .enabled()
            .get("draft/relaymsg")
            .and_then(Option::as_deref)
            .unwrap_or("/");
        if !nick_with_separator
            .chars()
            .any(|separator| separators.contains(separator))
        {
            return Err(RequestError::InvalidRelayNickname);
        }
        self.request_for_feature(
            now,
            Message::new("RELAYMSG", [channel, nick_with_separator, text]),
            Feature::Relay,
        )
    }
}
