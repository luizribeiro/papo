use std::time::Instant;

use papo_proto::Message;

use crate::{Feature, PersistenceSetting, RequestError, Session};

impl Session {
    /// Requests the current `draft/persistence` status.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when the capability was not negotiated. Other
    /// failures are the same as [`Session::request`].
    pub fn persistence_get(&mut self, now: Instant) -> Result<(), RequestError> {
        self.persistence_request(now, Message::new("PERSISTENCE", ["GET"]))
    }

    /// Changes the account's `draft/persistence` preference.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when the capability was not negotiated. Other
    /// failures are the same as [`Session::request`].
    pub fn persistence_set(
        &mut self,
        now: Instant,
        setting: PersistenceSetting,
    ) -> Result<(), RequestError> {
        self.persistence_request(now, Message::new("PERSISTENCE", ["SET", setting.as_str()]))
    }

    fn persistence_request(&mut self, now: Instant, message: Message) -> Result<(), RequestError> {
        if !self.caps.enabled().contains_key("draft/persistence") {
            return Err(RequestError::unsupported(Feature::Persistence));
        }
        self.request(now, message)?;
        Ok(())
    }
}
