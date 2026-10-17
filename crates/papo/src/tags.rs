//! Well-known IRC message-tag names.

/// Well-known client-only tag keys.
pub mod keys {
    /// Reply reference tag.
    pub const REPLY: &str = "+reply";
    /// Legacy reply reference tag.
    pub const DRAFT_REPLY: &str = "+draft/reply";
    /// Reaction-addition tag.
    pub const DRAFT_REACT: &str = "+draft/react";
    /// Reaction-removal tag.
    pub const DRAFT_UNREACT: &str = "+draft/unreact";
    /// Typing-state tag.
    pub const TYPING: &str = "+typing";
    /// Channel-context tag.
    pub const CHANNEL_CONTEXT: &str = "+channel-context";
    /// Legacy channel-context tag.
    pub const DRAFT_CHANNEL_CONTEXT: &str = "+draft/channel-context";
}
