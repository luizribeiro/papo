//! Registered metadata key names.

/// Registered metadata keys.
pub mod keys {
    use crate::MetadataKey;

    /// Avatar image URL.
    pub const AVATAR: MetadataKey = MetadataKey::__from_static("avatar");
    /// Display name.
    pub const DISPLAY_NAME: MetadataKey = MetadataKey::__from_static("display-name");
    /// Whether the user or channel is a bot.
    pub const BOT: MetadataKey = MetadataKey::__from_static("bot");
    /// Display color.
    pub const COLOR: MetadataKey = MetadataKey::__from_static("color");
    /// Homepage URL.
    pub const HOMEPAGE: MetadataKey = MetadataKey::__from_static("homepage");
    /// Pronouns.
    pub const PRONOUNS: MetadataKey = MetadataKey::__from_static("pronouns");
    /// Free-form status text.
    pub const STATUS: MetadataKey = MetadataKey::__from_static("status");
    /// Whether a user or channel is pinned in a Soju-compatible client.
    #[cfg(feature = "unstable")]
    pub const PINNED: MetadataKey = MetadataKey::__from_static("soju.im/pinned");
    /// Whether a user or channel is muted in a Soju-compatible client.
    #[cfg(feature = "unstable")]
    pub const MUTED: MetadataKey = MetadataKey::__from_static("soju.im/muted");
    /// Whether a user or channel is blocked in a Soju-compatible client.
    #[cfg(feature = "unstable")]
    pub const BLOCKED: MetadataKey = MetadataKey::__from_static("soju.im/blocked");
}

#[cfg(all(test, not(browser)))]
mod tests {
    #[test]
    fn registered_keys_match_the_registry() {
        let _: crate::MetadataKey = super::keys::AVATAR;
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
