use web_time::Instant;

use papo_proto::Line;

use crate::{Feature, PersistenceSetting, RequestError, Session};

impl Session {
    /// Requests the current `draft/persistence` status.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when the capability was not negotiated. Other
    /// failures are the same as [`Session::request`].
    pub fn persistence_get(&mut self, now: Instant) -> Result<(), RequestError> {
        self.persistence_request(now, Line::new("PERSISTENCE", ["GET"]))
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
        self.persistence_request(now, Line::new("PERSISTENCE", ["SET", setting.as_str()]))
    }

    fn persistence_request(&mut self, now: Instant, message: Line) -> Result<(), RequestError> {
        self.request_for_feature(now, message, Feature::Persistence)?;
        Ok(())
    }
}
