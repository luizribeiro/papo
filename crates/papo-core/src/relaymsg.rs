use std::time::Instant;

use papo_proto::Message;

use crate::{RequestError, RequestId, Session};

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
        let Some(value) = self.caps.enabled().get("draft/relaymsg") else {
            return Err(RequestError::Unsupported("relaymsg"));
        };
        let separators = value.as_deref().unwrap_or("/");
        if !nick_with_separator
            .chars()
            .any(|separator| separators.contains(separator))
        {
            return Err(RequestError::InvalidRelayNickname);
        }
        self.request(
            now,
            Message::new("RELAYMSG", [channel, nick_with_separator, text]),
        )
    }
}
