use std::time::Duration;

/// A policy controlling retries after an IRC connection ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reconnect {
    max_delay: Duration,
    max_attempts: Option<usize>,
}

impl Reconnect {
    /// Retries indefinitely with jittered exponential backoff.
    #[must_use]
    pub const fn exponential() -> Self {
        Self {
            max_delay: Duration::from_mins(5),
            max_attempts: None,
        }
    }

    /// Disables reconnection.
    #[must_use]
    pub const fn never() -> Self {
        Self {
            max_delay: Duration::ZERO,
            max_attempts: Some(1),
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

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the driver consumes policies when reconnecting")
    )]
    pub(crate) fn permits(self, completed_attempts: usize) -> bool {
        self.max_attempts
            .is_none_or(|maximum| completed_attempts < maximum)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the driver consumes schedules when reconnecting")
    )]
    pub(crate) fn delay(self, retry: u32, entropy: u64) -> Duration {
        let seconds = 1_u64.checked_shl(retry.min(63)).unwrap_or(u64::MAX);
        let base = Duration::from_secs(seconds).min(self.max_delay);
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

#[cfg(test)]
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
    fn never_policy_refuses_a_second_attempt() {
        let policy = Reconnect::never();
        assert!(policy.permits(0));
        assert!(!policy.permits(1));
    }
}
