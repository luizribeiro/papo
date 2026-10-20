use web_time::Instant;

use papo_proto::Line;

use crate::{Feature, RequestError, RequestId, Session};

impl Session {
    /// Requests that a joined channel be renamed.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when `draft/channel-rename` was not negotiated.
    /// Other failures are the same as [`Session::request`].
    pub fn rename(
        &mut self,
        now: Instant,
        old: &str,
        new: &str,
        reason: &str,
    ) -> Result<RequestId, RequestError> {
        let mut params = vec![old, new];
        if !reason.is_empty() {
            params.push(reason);
        }
        self.request_for_feature(now, Line::new("RENAME", params), Feature::ChannelRename)
    }
}
