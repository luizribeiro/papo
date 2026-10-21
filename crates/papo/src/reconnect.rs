use std::time::Duration;

/// A policy controlling retries after an IRC connection ends.
///
/// papo deliberately stops for a fixed set of permanent connection failures
/// that is stricter than the policies used by some IRC libraries. This safety
/// policy is not configurable. An application may call [`Client::reconnect`](crate::Client::reconnect)
/// after the final [`Event::Disconnected`](crate::Event::Disconnected) if it
/// has outside evidence that another attempt is appropriate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reconnect {
    max_delay: Duration,
    max_attempts: Option<usize>,
    refused_delay: Duration,
}

impl Reconnect {
    /// Retries indefinitely with jittered exponential backoff.
    #[must_use]
    pub const fn exponential() -> Self {
        Self {
            max_delay: Duration::from_mins(5),
            max_attempts: None,
            refused_delay: Duration::from_secs(60),
        }
    }

    /// Disables reconnection.
    #[must_use]
    pub const fn never() -> Self {
        Self {
            max_delay: Duration::ZERO,
            max_attempts: Some(1),
            refused_delay: Duration::from_secs(60),
        }
    }

    /// Caps the unjittered exponential delay.
    #[must_use]
    pub const fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Limits the total number of connection attempts, including the first.
    #[must_use]
    pub const fn max_attempts(mut self, attempts: usize) -> Self {
        self.max_attempts = Some(attempts);
        self
    }

    /// Sets the minimum delay after a server refuses IRC login.
    ///
    /// The default is 60 seconds. This delay also applies when required SASL
    /// is unavailable or times out, and when every nickname candidate is
    /// rejected. It takes precedence over a smaller [`Self::max_delay`].
    #[must_use]
    pub const fn refused_delay(mut self, delay: Duration) -> Self {
        self.refused_delay = delay;
        self
    }

    pub(crate) fn permits(self, completed_attempts: usize) -> bool {
        self.max_attempts
            .is_none_or(|maximum| completed_attempts < maximum)
    }

    pub(crate) fn delay(self, retry: u32, entropy: u64) -> Duration {
        Self::jitter(Duration::from_secs(1), retry, self.max_delay, entropy)
    }

    pub(crate) fn refused(self, retry: u32, entropy: u64) -> Duration {
        Self::jitter(
            self.refused_delay,
            retry,
            self.max_delay.max(self.refused_delay),
            entropy,
        )
        .max(self.refused_delay)
    }

    fn jitter(initial: Duration, retry: u32, maximum: Duration, entropy: u64) -> Duration {
        let multiplier = 1_u32.checked_shl(retry.min(31)).unwrap_or(u32::MAX);
        let base = initial.saturating_mul(multiplier).min(maximum);
        let denominator = u128::from(u64::MAX) * 100;
        let numerator = u128::from(u64::MAX) * 80 + u128::from(entropy) * 40;
        let nanos = base.as_nanos().saturating_mul(numerator) / denominator;
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

impl Default for Reconnect {
    fn default() -> Self {
        Self::exponential()
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::Duration;

    use super::Reconnect;

    #[test]
    fn exponential_schedule_doubles_and_caps_before_jitter() {
        let policy = Reconnect::exponential();
        assert_eq!(policy.delay(0, 0), Duration::from_millis(800));
        assert_eq!(policy.delay(1, u64::MAX), Duration::from_millis(2400));
        assert_eq!(policy.delay(2, 0), Duration::from_millis(3200));
        assert_eq!(policy.delay(20, u64::MAX), Duration::from_secs(360));
    }

    #[test]
    fn custom_limits_control_delay_and_attempts() {
        let policy = Reconnect::exponential()
            .max_delay(Duration::from_secs(10))
            .max_attempts(3);
        assert!(policy.permits(2));
        assert!(!policy.permits(3));
        assert_eq!(policy.delay(8, 0), Duration::from_secs(8));
    }

    #[test]
    fn refusals_start_at_sixty_seconds_then_back_off() {
        let policy = Reconnect::exponential();
        assert_eq!(policy.refused(0, 0), Duration::from_secs(60));
        assert_eq!(policy.refused(1, 0), Duration::from_secs(96));
        assert_eq!(policy.refused(1, u64::MAX), Duration::from_secs(144));
        assert_eq!(policy.refused(20, u64::MAX), Duration::from_secs(360));
    }

    #[test]
    fn refusal_minimum_overrides_a_smaller_normal_cap() {
        let policy = Reconnect::exponential()
            .max_delay(Duration::from_secs(10))
            .refused_delay(Duration::from_secs(75));
        assert_eq!(policy.refused(0, 0), Duration::from_secs(75));
        assert_eq!(policy.refused(1, u64::MAX), Duration::from_secs(90));
    }

    #[test]
    fn never_policy_refuses_a_second_attempt() {
        let policy = Reconnect::never();
        assert!(policy.permits(0));
        assert!(!policy.permits(1));
    }
}
