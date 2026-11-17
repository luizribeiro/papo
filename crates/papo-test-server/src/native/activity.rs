use std::{fmt, time::Duration};

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
}
