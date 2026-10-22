/// An account's configured persistence preference.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum PersistenceSetting {
    /// Keep the account's client active while disconnected.
    On,
    /// Disconnect the account's client normally.
    Off,
    /// Use the server's default persistence policy.
    Default,
}

impl PersistenceSetting {
    /// Returns the wire token for this setting.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "ON",
            Self::Off => "OFF",
            Self::Default => "DEFAULT",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "ON" => Some(Self::On),
            "OFF" => Some(Self::Off),
            "DEFAULT" => Some(Self::Default),
            _ => None,
        }
    }
}

/// Server-reported account persistence state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PersistenceStatus {
    /// The account's configured preference.
    pub setting: PersistenceSetting,
    /// Whether persistence is currently effective.
    pub effective: bool,
}
