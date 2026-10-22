use web_time::Instant;

use papo_proto::Line;

use crate::{Feature, RequestError, RequestId, Session, cap};

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
        self.validate_target(channel)?;
        let separators = self
            .caps
            .enabled()
            .get(&cap::RELAYMSG)
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
            Line::builder("RELAYMSG")
                .param(channel)
                .param(nick_with_separator)
                .param(text),
            Feature::Relay,
        )
    }
}
