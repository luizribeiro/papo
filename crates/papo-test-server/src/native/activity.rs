use std::{fmt, iter::Sum, ops::Add, time::Duration};

#[cfg_attr(target_os = "linux", path = "activity/linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "activity/other.rs")]
mod platform;

pub(crate) use platform::of;

/// What a server process was doing when it was sampled.
#[derive(Clone, Copy, Default)]
pub(crate) struct Activity {
    /// Whether a thread was running, waiting for a CPU, or waiting on the disk.
    pub(crate) busy: bool,
    /// The CPU time its threads have used so far.
    pub(crate) cpu: Duration,
}

/// Combines the samples of several threads or processes: busy if any of them is.
impl Add for Activity {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            busy: self.busy || other.busy,
            cpu: self.cpu + other.cpu,
        }
    }
}

impl Sum for Activity {
    fn sum<I: Iterator<Item = Self>>(samples: I) -> Self {
        samples.fold(Self::default(), Add::add)
    }
}

impl fmt::Display for Activity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.busy { "busy" } else { "idle" };
        write!(formatter, "{state}, {:.1} s CPU", self.cpu.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Activity;

    #[test]
    fn shows_the_state_and_cpu_time() {
        let starved = Activity {
            busy: true,
            cpu: Duration::from_millis(1_340),
        };
        assert_eq!(starved.to_string(), "busy, 1.3 s CPU");
        assert_eq!(Activity::default().to_string(), "idle, 0.0 s CPU");
    }

    #[test]
    fn a_sum_is_busy_if_any_part_is_and_adds_their_cpu_time() {
        let sample = |busy, millis| Activity {
            busy,
            cpu: Duration::from_millis(millis),
        };
        let total = |samples: &[Activity]| samples.iter().copied().sum::<Activity>().to_string();

        assert_eq!(
            total(&[sample(false, 120), sample(false, 40)]),
            "idle, 0.2 s CPU"
        );
        assert_eq!(
            total(&[sample(false, 400), sample(true, 900)]),
            "busy, 1.3 s CPU"
        );
        assert_eq!(
            total(&[sample(true, 0), sample(false, 0)]),
            "busy, 0.0 s CPU"
        );
        assert_eq!(total(&[]), "idle, 0.0 s CPU");
    }
}
