use std::time::{Duration, Instant};

use super::activity::Activity;

const IDLE_LIMIT: Duration = Duration::from_secs(10);

/// Bounds a server's start by the time it spends idle, so a loaded machine only slows it down.
pub(crate) struct StartupBudget {
    launched: Instant,
    sampled: Instant,
    idle: Duration,
}

impl StartupBudget {
    pub(crate) const fn new(now: Instant) -> Self {
        Self {
            launched: now,
            sampled: now,
            idle: Duration::ZERO,
        }
    }

    pub(crate) fn observe(&mut self, now: Instant, activity: Activity) -> Result<(), String> {
        if !activity.busy {
            self.idle += now.saturating_duration_since(self.sampled);
        }
        self.sampled = now;
        if self.idle < IDLE_LIMIT {
            return Ok(());
        }
        Err(format!(
            "stayed idle before becoming ready ({:.1} s idle, {:.1} s since launch)",
            self.idle.as_secs_f64(),
            now.saturating_duration_since(self.launched).as_secs_f64(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::StartupBudget;
    use crate::native::activity::Activity;

    const IDLE: Activity = Activity {
        busy: false,
        cpu: Duration::ZERO,
    };
    const STARVED: Activity = Activity {
        busy: true,
        cpu: Duration::ZERO,
    };

    #[test]
    fn gives_up_after_ten_idle_seconds() {
        let launch = Instant::now();
        let mut budget = StartupBudget::new(launch);
        let at = |millis| launch + Duration::from_millis(millis);

        assert!(budget.observe(at(9_999), IDLE).is_ok());
        assert_eq!(
            budget.observe(at(10_000), IDLE),
            Err("stayed idle before becoming ready (10.0 s idle, 10.0 s since launch)".into())
        );
    }

    #[test]
    fn time_spent_busy_is_not_idle() {
        let launch = Instant::now();
        let mut budget = StartupBudget::new(launch);
        let at = |millis| launch + Duration::from_millis(millis);

        assert!(budget.observe(at(60_000), STARVED).is_ok());
        assert!(budget.observe(at(69_999), IDLE).is_ok());
        assert_eq!(
            budget.observe(at(70_000), IDLE),
            Err("stayed idle before becoming ready (10.0 s idle, 70.0 s since launch)".into())
        );
    }
}
