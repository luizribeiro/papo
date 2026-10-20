use super::{Event, LineRef, Outbox, PersistenceSetting, PersistenceStatus, State};

impl State {
    #[cfg(feature = "unstable")]
    pub(super) fn handle_persistence(
        &mut self,
        message: &LineRef<'_>,
        outbox: &mut Outbox,
    ) -> bool {
        if !message.command_is("PERSISTENCE") || message.param(0) != Some("STATUS") {
            return false;
        }
        let (Some(setting), Some(effective)) = (message.param(1), message.param(2)) else {
            return false;
        };
        let (Some(setting), Some(effective)) = (
            PersistenceSetting::parse(setting),
            match effective.to_ascii_uppercase().as_str() {
                "ON" => Some(true),
                "OFF" => Some(false),
                _ => None,
            },
        ) else {
            return false;
        };
        self.persistence = Some(PersistenceStatus { setting, effective });
        outbox.event(Event::PersistenceStatus { setting, effective });
        true
    }

    /// Returns the server-reported account persistence status.
    #[must_use]
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const fn persistence(&self) -> Option<PersistenceStatus> {
        self.persistence
    }
}
