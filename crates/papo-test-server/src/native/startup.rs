use std::time::{Duration, Instant};

use super::activity::Activity;

const IDLE_LIMIT: Duration = Duration::from_secs(10);
const CPU_LIMIT: Duration = Duration::from_secs(30);

/// Bounds a server's start by its idle time and CPU use, so a loaded machine only slows it down.
pub(crate) struct StartupBudget {
    launched: Instant,
    sampled: Instant,
    cpu: Duration,
    idle: Duration,
}

impl StartupBudget {
    pub(crate) const fn new(now: Instant) -> Self {
        Self {
            launched: now,
            sampled: now,
            cpu: Duration::ZERO,
            idle: Duration::ZERO,
        }
    }

    /// Charges the time since the last sample as idle unless the child was busy or used CPU.
    pub(crate) fn observe(&mut self, now: Instant, activity: Activity) -> Result<(), String> {
        if !activity.busy && activity.cpu <= self.cpu {
            self.idle += now.saturating_duration_since(self.sampled);
        }
        self.sampled = now;
        self.cpu = self.cpu.max(activity.cpu);
        let reason = if self.cpu >= CPU_LIMIT {
            "used too much CPU"
        } else if self.idle >= IDLE_LIMIT {
            "stayed idle"
        } else {
            return Ok(());
        };
        Err(format!(
            "{reason} before becoming ready ({:.1} s idle, {:.1} s CPU, {:.1} s since launch)",
            self.idle.as_secs_f64(),
            self.cpu.as_secs_f64(),
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
            Err(
                "stayed idle before becoming ready (10.0 s idle, 0.0 s CPU, 10.0 s since launch)"
                    .into()
            )
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
            Err(
                "stayed idle before becoming ready (10.0 s idle, 0.0 s CPU, 70.0 s since launch)"
                    .into()
            )
        );
    }

    #[test]
    fn cpu_used_between_samples_is_not_idle() {
        let launch = Instant::now();
        let mut budget = StartupBudget::new(launch);
        let ran = |millis| Activity {
            busy: false,
            cpu: Duration::from_millis(millis),
        };

        assert!(
            budget
                .observe(launch + Duration::from_secs(60), ran(1_250))
                .is_ok()
        );
        assert!(
            budget
                .observe(launch + Duration::from_secs(69), ran(1_250))
                .is_ok()
        );
        assert!(
            budget
                .observe(launch + Duration::from_secs(70), ran(1_250))
                .is_err()
        );
    }

    #[test]
    fn gives_up_after_thirty_cpu_seconds() {
        let launch = Instant::now();
        let mut budget = StartupBudget::new(launch);
        let spinning = |millis| Activity {
            busy: true,
            cpu: Duration::from_millis(millis),
        };

        assert!(
            budget
                .observe(launch + Duration::from_secs(1), spinning(29_990))
                .is_ok()
        );
        assert_eq!(
            budget.observe(launch + Duration::from_secs(2), spinning(30_000)),
            Err("used too much CPU before becoming ready (0.0 s idle, 30.0 s CPU, 2.0 s since launch)".into())
        );
    }
}
