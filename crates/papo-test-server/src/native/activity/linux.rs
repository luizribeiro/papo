use std::{fs, time::Duration};

use super::Activity;

/// Linux reports CPU time in `USER_HZ` ticks, a fixed 100 per second on every supported architecture.
const TICKS_PER_SECOND: u64 = 100;

/// Samples every thread of `pid`; a process that cannot be read looks idle.
pub(crate) fn of(pid: u32) -> Activity {
    let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return Activity::default();
    };
    tasks
        .flatten()
        .filter_map(|task| parse(&fs::read_to_string(task.path().join("stat")).ok()?))
        .sum()
}

fn parse(stat: &str) -> Option<Activity> {
    let (_, fields) = stat.rsplit_once(')')?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    let ticks = |index: usize| fields.get(index)?.parse::<u64>().ok();
    Some(Activity {
        busy: matches!(*fields.first()?, "R" | "D"),
        cpu: Duration::from_millis((ticks(11)? + ticks(12)?) * 1_000 / TICKS_PER_SECOND),
    })
}

#[cfg(test)]
mod tests {
    use std::{process::Command, time::Duration};

    use super::{of, parse};

    const ERGO_THREAD: &str = "2892229 (ergo) S 2892216 2892052 2892052 0 -1 1077936192 1172 0 0 0 7 3 0 0 20 0 10 0 38803260 1988005888 4219 18446744073709551615";

    #[test]
    fn reads_state_and_cpu_time_past_a_name_with_parentheses() {
        let thread = parse(&ERGO_THREAD.replace("(ergo)", "(ergo (x) R)")).unwrap();
        assert!(!thread.busy);
        assert_eq!(thread.cpu, Duration::from_millis(100));

        for (state, busy) in [
            ("R", true),
            ("D", true),
            ("S", false),
            ("T", false),
            ("Z", false),
        ] {
            let line = ERGO_THREAD.replace(") S ", &format!(") {state} "));
            assert_eq!(parse(&line).unwrap().busy, busy, "state {state}");
        }
    }

    #[test]
    fn rejects_a_truncated_stat_line() {
        assert!(parse("2892229 (ergo) S 2892216 2892052").is_none());
    }

    #[test]
    fn a_reading_thread_is_busy_and_a_sleeping_process_settles_idle() {
        assert!(of(std::process::id()).busy);

        let mut sleeping = Command::new("sleep").arg("60").spawn().unwrap();
        while of(sleeping.id()).busy {
            std::thread::yield_now();
        }
        sleeping.kill().unwrap();
        sleeping.wait().unwrap();
    }

    #[test]
    fn a_process_uses_at_least_the_cpu_time_of_its_spinning_thread() {
        let spun = loop {
            let spun = parse(&std::fs::read_to_string("/proc/thread-self/stat").unwrap()).unwrap();
            if spun.cpu > Duration::ZERO {
                break spun.cpu;
            }
        };
        assert!(of(std::process::id()).cpu >= spun);
    }

    #[test]
    fn an_unknown_process_looks_idle() {
        let mut exited = Command::new("true").spawn().unwrap();
        exited.wait().unwrap();
        let activity = of(exited.id());
        assert!(!activity.busy && activity.cpu == Duration::ZERO);
    }
}
