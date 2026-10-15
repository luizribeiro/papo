use crate::Client;

use super::{BotConfig, ReplyRateLimiter};

/// An extractor-style command router.
pub struct Bot {
    client: Client,
    config: BotConfig,
}

impl Bot {
    /// Creates a router for `client` using `!` commands and mention addressing.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            config: BotConfig::new("!"),
        }
    }

    /// Changes the command prefix.
    #[must_use]
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.config.prefix = prefix.into();
        self
    }

    /// Enables or disables mention addressing.
    #[must_use]
    pub fn mention(mut self, enabled: bool) -> Self {
        self.config.mention = enabled;
        self
    }

    /// Replaces the limiter shared by command responses.
    #[must_use]
    pub fn reply_limiter(mut self, limiter: ReplyRateLimiter) -> Self {
        self.config.replies = limiter;
        self
    }

    /// Returns the client kept alive by this router.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }
}
