use std::time::Duration;

use papo_proto::ServerTime;
use web_time::Instant;

/// Covers ordinary timestamp-to-receipt delay without accepting a value one second ahead.
const TOLERANCE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug)]
pub(super) struct ServerClock {
    newest: ServerTime,
    observed_at: Instant,
}

impl ServerClock {
    pub(super) const fn new(newest: ServerTime, observed_at: Instant) -> Self {
        Self {
            newest,
            observed_at,
        }
    }

    pub(super) const fn newest(self) -> ServerTime {
        self.newest
    }

    pub(super) fn estimated_now(self, now: Instant) -> ServerTime {
        let elapsed = now.saturating_duration_since(self.observed_at) + TOLERANCE;
        let millis = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
        ServerTime::from_stored_millis(self.newest.as_millis().saturating_add(millis))
            .unwrap_or(self.newest)
    }
}

#[cfg(test)]
mod tests {
    use super::ServerClock;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use web_time::Instant;

    #[test]
    fn estimate_uses_only_monotonic_elapsed_time_and_tolerance() {
        let now = Instant::now();
        let seen = papo_proto::ServerTime::from_stored_millis(42_000).unwrap();
        let clock = ServerClock::new(seen, now);
        assert_eq!(clock.estimated_now(now).as_millis(), 42_250);
        let later = now.checked_add(Duration::from_secs(3)).unwrap();
        assert_eq!(clock.estimated_now(later).as_millis(), 45_250);
    }

    #[test]
    fn device_wall_clock_skew_cannot_change_the_estimate() {
        let device_millis = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let six_hours = 6 * 60 * 60 * 1_000;
        for server_millis in [device_millis - six_hours, device_millis + six_hours] {
            let now = Instant::now();
            let server_time = papo_proto::ServerTime::from_stored_millis(server_millis).unwrap();
            let estimate = ServerClock::new(server_time, now).estimated_now(now);
            assert_eq!(estimate.as_millis(), server_millis + 250);
        }
    }
}
