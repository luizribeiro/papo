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
    /// Whether a target is pinned in a Soju-compatible client.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const PINNED: &str = "soju.im/pinned";
    /// Whether a target is muted in a Soju-compatible client.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const MUTED: &str = "soju.im/muted";
    /// Whether a target is blocked in a Soju-compatible client.
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub const BLOCKED: &str = "soju.im/blocked";
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

    #[cfg(feature = "unstable")]
    #[test]
    fn vendor_keys_match_soju() {
        assert_eq!(super::keys::PINNED, "soju.im/pinned");
        assert_eq!(super::keys::MUTED, "soju.im/muted");
        assert_eq!(super::keys::BLOCKED, "soju.im/blocked");
    }
}
