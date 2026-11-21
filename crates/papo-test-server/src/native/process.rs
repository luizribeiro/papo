use std::{
    fmt,
    io::{self, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use super::{activity, startup::StartupBudget};

#[cfg_attr(unix, path = "process/unix.rs")]
#[cfg_attr(not(unix), path = "process/other.rs")]
mod platform;

const STDERR_LIMIT: usize = 4096;
const START_ATTEMPTS: usize = 3;
pub(crate) struct ServerChild {
    child: Child,
    stderr: Arc<Mutex<Tail>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl ServerChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        platform::prepare(command);
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut pipe = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("server stderr pipe is unavailable"))?;
        let stderr = Arc::new(Mutex::new(Tail::default()));
        let captured = Arc::clone(&stderr);
        let reader = thread::spawn(move || {
            let mut chunk = [0; 1024];
            while let Ok(count) = pipe.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                if let Ok(mut tail) = captured.lock() {
                    tail.extend(&chunk[..count]);
                }
            }
        });
        Ok(Self {
            child,
            stderr,
            reader: Some(reader),
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn try_wait(&mut self, name: &str) -> io::Result<Option<ExitStatus>> {
        if self.reader.is_none() || platform::exited(&mut self.child, false)? {
            let status = self.stop_and_wait()?;
            return Err(self.failure(io::ErrorKind::Other, format!("{name} exited with {status}")));
        }
        Ok(None)
    }

    /// Waits for a one-shot child, failing with its stderr tail unless it succeeds.
    pub(crate) fn finish(mut self, name: &str) -> io::Result<()> {
        drop(self.child.stdin.take());
        if self.reader.is_some() {
            platform::exited(&mut self.child, true)?;
        }
        let status = self.stop_and_wait()?;
        if status.success() {
            return Ok(());
        }
        Err(self.failure(io::ErrorKind::Other, format!("{name} exited with {status}")))
    }

    pub(crate) fn check(&mut self, name: &str) -> io::Result<()> {
        self.try_wait(name).map(|_| ())
    }

    /// Fails once the child has spent its startup budget without becoming ready.
    pub(crate) fn charge(&self, name: &str, budget: &mut StartupBudget) -> io::Result<()> {
        self.charge_activity(name, budget, activity::of(self.id()))
    }

    /// Like [`Self::charge`] for a child whose readiness also waits on `peer`, so progress by
    /// either one keeps the start alive.
    pub(crate) fn charge_alongside(
        &self,
        peer: &Self,
        name: &str,
        budget: &mut StartupBudget,
    ) -> io::Result<()> {
        let activity = [self.id(), peer.id()].into_iter().map(activity::of).sum();
        self.charge_activity(name, budget, activity)
    }

    fn charge_activity(
        &self,
        name: &str,
        budget: &mut StartupBudget,
        activity: activity::Activity,
    ) -> io::Result<()> {
        budget.observe(Instant::now(), activity).map_err(|reason| {
            self.failure(
                io::ErrorKind::TimedOut,
                format!("{name} did not start: {reason}"),
            )
        })
    }

    pub(crate) fn stop(&mut self) -> io::Result<()> {
        self.stop_and_wait().map(|_| ())
    }

    fn stop_and_wait(&mut self) -> io::Result<ExitStatus> {
        if self.reader.is_some() {
            platform::kill(&mut self.child)?;
        }
        let status = self.child.wait()?;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        Ok(status)
    }

    fn failure(&self, kind: io::ErrorKind, summary: String) -> io::Error {
        let stderr = self
            .stderr
            .lock()
            .map_or_else(|_| String::new(), |tail| tail.text());
        io::Error::new(kind, Failure::new(summary, stderr))
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub(crate) fn retry_start<T>(mut start: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for attempt in 1..=START_ATTEMPTS {
        match start() {
            Err(error) if attempt < START_ATTEMPTS && retryable(&error) => {}
            result => return result,
        }
    }
    unreachable!()
}

fn retryable(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AddrInUse
        || error
            .get_ref()
            .and_then(|source| source.downcast_ref::<Failure>())
            .is_some_and(|failure| failure.retryable)
}

struct Failure {
    summary: String,
    stderr: String,
    retryable: bool,
}

impl Failure {
    fn new(summary: String, stderr: String) -> Self {
        let diagnostic = stderr.to_ascii_lowercase();
        let retryable = ["address already in use", "address in use", "eaddrinuse"]
            .iter()
            .any(|phrase| diagnostic.contains(phrase));
        Self {
            summary,
            stderr,
            retryable,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.summary)?;
        if !self.stderr.is_empty() {
            write!(formatter, ": {}", self.stderr.trim())?;
        }
        Ok(())
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for Failure {}

#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn extend(&mut self, input: &[u8]) {
        self.0.extend(input);
        let excess = self.0.len().saturating_sub(STDERR_LIMIT);
        self.0.drain(..excess);
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::{io, process::Command};

    use super::{Failure, STDERR_LIMIT, ServerChild, Tail, platform, retry_start};
    use crate::native::startup::tests::settled_budget;

    #[test]
    fn only_an_idle_child_spends_its_startup_budget() {
        let busy =
            ServerChild::spawn(Command::new("sh").args(["-c", "while :; do :; done"])).unwrap();
        assert!(
            busy.charge("Ergo", &mut settled_budget(&[busy.id()], true))
                .is_ok()
        );

        let idle = ServerChild::spawn(Command::new("sleep").arg("60")).unwrap();
        let error = idle
            .charge("Ergo", &mut settled_budget(&[idle.id()], false))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            error
                .to_string()
                .starts_with("Ergo did not start: stayed idle before becoming ready (11.0 s idle")
        );
    }

    #[test]
    fn an_idle_child_waiting_on_a_busy_peer_keeps_its_startup_budget() {
        let idle = ServerChild::spawn(Command::new("sleep").arg("60")).unwrap();
        settled_budget(&[idle.id()], false);
        let busy =
            ServerChild::spawn(Command::new("sh").args(["-c", "while :; do :; done"])).unwrap();
        let mut budget = settled_budget(&[idle.id(), busy.id()], true);
        assert!(idle.charge_alongside(&busy, "Atheme", &mut budget).is_ok());
        let mut budget = settled_budget(&[busy.id(), idle.id()], true);
        assert!(busy.charge_alongside(&idle, "Atheme", &mut budget).is_ok());

        let peer = ServerChild::spawn(Command::new("sleep").arg("60")).unwrap();
        let mut budget = settled_budget(&[idle.id(), peer.id()], false);
        let error = idle
            .charge_alongside(&peer, "Atheme", &mut budget)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            error.to_string().starts_with(
                "Atheme did not start: stayed idle before becoming ready (11.0 s idle"
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn teardown_stops_helpers_that_keep_stderr_open() {
        use std::process::Stdio;
        use std::sync::mpsc;
        use std::thread;
        use std::time::{Duration, Instant};

        type Teardown = fn(ServerChild);
        let cases: [(&str, Teardown); 3] = [
            ("stop", |mut child| {
                child.check("Solanum").unwrap();
                child.stop().unwrap();
            }),
            ("check", |mut child| {
                while child.check("Solanum").is_ok() {
                    thread::yield_now();
                }
            }),
            ("finish", |child| child.finish("ergo initdb").unwrap()),
        ];
        for (mode, teardown) in cases {
            let script = "exec 3<&0
                          sh -c 'echo helper ready >&2; read -r _ <&3' &
                          case $mode in stop) wait ;; check) exit 1 ;; esac";
            let mut child = ServerChild::spawn(
                Command::new("sh")
                    .args(["-c", script])
                    .env("mode", mode)
                    .stdin(Stdio::piped()),
            )
            .unwrap();
            let release = child.child.stdin.take().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !child.stderr.lock().unwrap().text().contains("helper ready") {
                assert!(Instant::now() < deadline, "helper did not start");
                thread::yield_now();
            }
            thread::scope(|scope| {
                let (send, recv) = mpsc::channel();
                scope.spawn(move || {
                    teardown(child);
                    let _ = send.send(());
                });
                let result = recv.recv_timeout(Duration::from_secs(10));
                drop(release);
                result.expect(mode);
            });
        }
    }

    #[test]
    fn child_id_matches_the_spawned_process() {
        let child = ServerChild::spawn(Command::new("sh").args(["-c", "exit 0"])).unwrap();

        assert_eq!(child.id(), child.child.id());
    }

    #[test]
    fn exited_child_reports_its_final_stderr_line() {
        let mut child = ServerChild::spawn(Command::new("sh").args([
            "-c",
            "printf 'loading ircd.yaml\\nlisten: address already in use\\n' >&2",
        ]))
        .unwrap();
        assert!(platform::exited(&mut child.child, true).unwrap());

        let error = child.try_wait("Ergo").unwrap_err();

        assert!(child.reader.is_none());
        assert!(
            error
                .to_string()
                .ends_with("listen: address already in use")
        );
    }

    #[test]
    fn a_finished_child_reports_failure_with_its_stderr() {
        let failed = ServerChild::spawn(
            Command::new("sh").args(["-c", "echo 'open ircd.db: permission denied' >&2; exit 1"]),
        )
        .unwrap();
        let succeeded = ServerChild::spawn(Command::new("sh").args(["-c", "exit 0"])).unwrap();

        assert_eq!(
            failed.finish("ergo initdb").unwrap_err().to_string(),
            "ergo initdb exited with exit status: 1: open ircd.db: permission denied"
        );
        succeeded.finish("ergo initdb").unwrap();
    }

    #[test]
    fn failure_debug_shows_the_cause_and_stderr() {
        let failure = Failure::new(
            "Solanum exited with exit status: 1".into(),
            "ERROR: binding listener: Address already in use".into(),
        );

        assert_eq!(
            format!("{failure:?}"),
            "Solanum exited with exit status: 1: ERROR: binding listener: Address already in use"
        );
    }

    #[test]
    fn retries_an_address_in_use_startup_exit() {
        let mut attempts = 0;
        let port = retry_start(|| {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::other(Failure::new(
                    "soju exited with exit status: 1".into(),
                    "listen tcp 127.0.0.1:43821: bind: address already in use".into(),
                )))
            } else {
                Ok(43822)
            }
        })
        .unwrap();

        assert_eq!((port, attempts), (43822, 2));
    }

    #[test]
    fn does_not_retry_a_configuration_error() {
        let mut attempts = 0;
        let error = retry_start::<()>(|| {
            attempts += 1;
            Err(io::Error::other(Failure::new(
                "InspIRCd exited with exit status: 1".into(),
                "unknown configuration directive: link".into(),
            )))
        })
        .unwrap_err();

        assert_eq!(attempts, 1);
        assert!(
            error
                .to_string()
                .contains("unknown configuration directive")
        );
    }

    #[test]
    fn exhausts_address_in_use_retries_with_the_last_diagnostic() {
        let mut attempts = 0;
        let error = retry_start::<()>(|| {
            attempts += 1;
            Err(io::Error::other(Failure::new(
                "soju exited with exit status: 1".into(),
                format!(
                    "listen tcp 127.0.0.1:{}: bind: address already in use",
                    43820 + attempts
                ),
            )))
        })
        .unwrap_err();

        assert_eq!(attempts, 3);
        assert_eq!(
            error.to_string(),
            "soju exited with exit status: 1: listen tcp 127.0.0.1:43823: bind: address already in use"
        );
    }

    #[test]
    fn bounds_stderr_that_keeps_arriving() {
        let mut tail = Tail::default();
        for ordinal in 0..10_000_u32 {
            tail.extend(format!("diagnostic-{ordinal:05}\n").as_bytes());
        }
        assert_eq!(tail.0.len(), STDERR_LIMIT);
        let output = tail.text();
        assert!(output.ends_with("diagnostic-09999\n") && !output.contains("diagnostic-00000"));
    }
}
