use std::time::{Duration, Instant};

use super::activity::Activity;

const IDLE_LIMIT: Duration = Duration::from_secs(10);
const CPU_LIMIT: Duration = Duration::from_secs(30);
/// A backstop for a child that looks busy forever, such as one stuck in disk wait.
const WALL_LIMIT: Duration = Duration::from_secs(300);

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
        let elapsed = now.saturating_duration_since(self.launched);
        let reason = if self.cpu >= CPU_LIMIT {
            "used too much CPU"
        } else if elapsed >= WALL_LIMIT {
            "kept starting too long"
        } else if self.idle >= IDLE_LIMIT {
            "stayed idle"
        } else {
            return Ok(());
        };
        Err(format!(
            "{reason} before becoming ready ({:.1} s idle, {:.1} s CPU, {:.1} s since launch)",
            self.idle.as_secs_f64(),
            self.cpu.as_secs_f64(),
            elapsed.as_secs_f64(),
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::{Duration, Instant};

    use super::StartupBudget;
    use crate::native::activity::{self, Activity};

    /// Waits until the children together sample as `busy`, then ages a budget eleven seconds from
    /// that sample.
    pub(crate) fn settled_budget(pids: &[u32], busy: bool) -> StartupBudget {
        let sample = || pids.iter().map(|pid| activity::of(*pid)).sum::<Activity>();
        while sample().busy != busy {
            std::thread::yield_now();
        }
        let launch = Instant::now().checked_sub(Duration::from_secs(11)).unwrap();
        let mut budget = StartupBudget::new(launch);
        budget.observe(launch, sample()).unwrap();
        budget
    }

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

    #[test]
    fn gives_up_five_minutes_after_launch_however_busy() {
        let launch = Instant::now();
        let mut budget = StartupBudget::new(launch);
        let stuck = Activity {
            busy: true,
            cpu: Duration::from_millis(80),
        };

        assert!(
            budget
                .observe(launch + Duration::from_millis(299_999), stuck)
                .is_ok()
        );
        assert_eq!(
            budget.observe(launch + Duration::from_secs(300), stuck),
            Err("kept starting too long before becoming ready (0.0 s idle, 0.1 s CPU, 300.0 s since launch)".into())
        );
    }

    #[test]
    fn reports_cpu_before_time_and_time_before_idleness() {
        let launch = Instant::now();
        let mut budget = StartupBudget::new(launch);
        let idle = |millis| Activity {
            busy: false,
            cpu: Duration::from_millis(millis),
        };

        let late = budget
            .observe(launch + Duration::from_secs(300), idle(0))
            .unwrap_err();
        let spun = budget
            .observe(launch + Duration::from_secs(301), idle(30_000))
            .unwrap_err();

        assert!(late.starts_with("kept starting too long"), "{late}");
        assert!(spun.starts_with("used too much CPU"), "{spun}");
    }
}
