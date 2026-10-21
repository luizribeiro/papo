use std::time::Duration;
use web_time::Instant;

/// Why an outbound rate-limit configuration is invalid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum InvalidRateLimit {
    /// The bucket cannot hold any tokens.
    EmptyBurst,
    /// Tokens would never be replenished.
    ZeroRefill,
}

impl std::fmt::Display for InvalidRateLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyBurst => formatter.write_str("rate-limit burst must be positive"),
            Self::ZeroRefill => formatter.write_str("rate-limit refill must be positive"),
        }
    }
}

impl std::error::Error for InvalidRateLimit {}

/// Outbound token-bucket configuration.
///
/// The default permits a burst of five lines and then two lines per second. A server's
/// `SAFERATE` token disables local pacing unless [`Self::strict`] is selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimit {
    burst: usize,
    refill: Duration,
    strict: bool,
}

impl RateLimit {
    /// Creates a bucket with `burst` initial tokens and one token per `refill`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRateLimit`] when `burst` or `refill` is zero.
    pub const fn new(burst: usize, refill: Duration) -> Result<Self, InvalidRateLimit> {
        if burst == 0 {
            return Err(InvalidRateLimit::EmptyBurst);
        }
        if refill.is_zero() {
            return Err(InvalidRateLimit::ZeroRefill);
        }
        Ok(Self {
            burst,
            refill,
            strict: false,
        })
    }

    /// Disables outbound rate limiting.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            burst: usize::MAX,
            refill: Duration::MAX,
            strict: false,
        }
    }

    /// Keeps this limit active when the server advertises `SAFERATE`.
    #[must_use]
    pub const fn strict(mut self) -> Self {
        self.strict = true;
        self
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            burst: 5,
            refill: Duration::from_millis(500),
            strict: false,
        }
    }
}

#[derive(Debug)]
pub(super) struct RateLimiter {
    config: RateLimit,
    tokens: usize,
    next_token: Option<Instant>,
    pub(super) now: Option<Instant>,
}

impl RateLimiter {
    pub(super) const fn new(config: RateLimit) -> Self {
        Self {
            tokens: config.burst,
            config,
            next_token: None,
            now: None,
        }
    }

    pub(super) fn reset(&mut self, now: Instant) {
        self.tokens = self.config.burst;
        self.set_next_token(None);
        self.now = Some(now);
    }

    pub(super) fn advance(&mut self, now: Instant) {
        self.now = Some(now);
        while self.next_token.is_some_and(|deadline| now >= deadline) {
            self.tokens += 1;
            let next_token = self.next_token.and_then(|deadline| {
                (self.tokens < self.config.burst).then(|| deadline + self.config.refill)
            });
            self.set_next_token(next_token);
        }
    }

    pub(super) fn take(&mut self, saferate: bool) -> bool {
        if self.config.burst == usize::MAX || (saferate && !self.config.strict) {
            return true;
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        if self.next_token.is_none() {
            let next_token = self.now.map(|now| now + self.config.refill);
            self.set_next_token(next_token);
        }
        true
    }

    fn set_next_token(&mut self, next_token: Option<Instant>) {
        debug_assert!(next_token.is_none() || self.tokens < self.config.burst);
        self.next_token = next_token;
    }

    pub(super) const fn next_deadline(&self, queued: bool, saferate: bool) -> Option<Instant> {
        if queued && (!saferate || self.config.strict) {
            self.next_token
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimit;
    use std::time::Duration;

    #[test]
    fn rejects_an_empty_burst() {
        let error = RateLimit::new(0, Duration::from_millis(500)).unwrap_err();
        assert_eq!(error, super::InvalidRateLimit::EmptyBurst);
        assert_eq!(error.to_string(), "rate-limit burst must be positive");
    }

    #[test]
    fn rejects_a_zero_refill_interval() {
        let error = RateLimit::new(5, Duration::ZERO).unwrap_err();
        assert_eq!(error, super::InvalidRateLimit::ZeroRefill);
        assert_eq!(error.to_string(), "rate-limit refill must be positive");
    }
}
