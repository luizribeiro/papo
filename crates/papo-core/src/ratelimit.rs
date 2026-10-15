use std::time::Duration;

/// Outbound token-bucket configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimit {
    burst: usize,
    refill: Duration,
}

impl RateLimit {
    /// Creates a bucket with `burst` initial tokens and one token per `refill`.
    ///
    /// # Panics
    ///
    /// Panics when `burst` is zero or `refill` is zero.
    #[must_use]
    pub const fn new(burst: usize, refill: Duration) -> Self {
        assert!(burst > 0, "rate-limit burst must be positive");
        assert!(!refill.is_zero(), "rate-limit refill must be positive");
        Self { burst, refill }
    }

    /// Disables outbound rate limiting.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            burst: usize::MAX,
            refill: Duration::MAX,
        }
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new(5, Duration::from_millis(500))
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimit;
    use std::time::Duration;

    #[test]
    #[should_panic(expected = "burst must be positive")]
    fn rejects_an_empty_burst() {
        let _ = RateLimit::new(0, Duration::from_millis(500));
    }

    #[test]
    #[should_panic(expected = "refill must be positive")]
    fn rejects_a_zero_refill_interval() {
        let _ = RateLimit::new(5, Duration::ZERO);
    }
}
