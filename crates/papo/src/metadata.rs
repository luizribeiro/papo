//! Metadata APIs and registered key names.

pub use crate::ext::metadata::Metadata;

/// Registered metadata keys.
pub mod keys {
    /// Avatar image URL.
    pub const AVATAR: &str = "avatar";
    /// Display name.
    pub const DISPLAY_NAME: &str = "display-name";
    /// Whether the target is a bot.
    pub const BOT: &str = "bot";
    /// Display color.
    pub const COLOR: &str = "color";
    /// Homepage URL.
    pub const HOMEPAGE: &str = "homepage";
    /// Pronouns.
    pub const PRONOUNS: &str = "pronouns";
    /// Free-form status text.
    pub const STATUS: &str = "status";
}

#[cfg(test)]
mod tests {
    #[test]
    fn registered_keys_match_the_registry() {
        assert_eq!(super::keys::AVATAR, "avatar");
        assert_eq!(super::keys::DISPLAY_NAME, "display-name");
        assert_eq!(super::keys::BOT, "bot");
        assert_eq!(super::keys::COLOR, "color");
        assert_eq!(super::keys::HOMEPAGE, "homepage");
        assert_eq!(super::keys::PRONOUNS, "pronouns");
        assert_eq!(super::keys::STATUS, "status");
    }
}
