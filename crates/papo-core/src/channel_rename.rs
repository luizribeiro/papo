use std::time::Instant;

use papo_proto::Message;

use crate::{RequestError, RequestId, Session};

/// A typed `FAIL RENAME` code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChannelRenameError {
    /// The destination channel name is already in use.
    ChannelNameInUse,
    /// The server does not permit this channel to be renamed.
    CannotRename,
}

impl ChannelRenameError {
    pub(crate) fn parse(command: &str, code: &str) -> Option<Self> {
        if !command.eq_ignore_ascii_case("RENAME") {
            return None;
        }
        match code.to_ascii_uppercase().as_str() {
            "CHANNEL_NAME_IN_USE" => Some(Self::ChannelNameInUse),
            "CANNOT_RENAME" => Some(Self::CannotRename),
            _ => None,
        }
    }
}

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
        if !self.caps.enabled().contains_key("draft/channel-rename") {
            return Err(RequestError::Unsupported("channel rename"));
        }
        self.request(now, Message::new("RENAME", [old, new, reason]))
    }
}
