use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

/// Observes a server's exit while allowing the owner to reap it during teardown.
///
/// The wait flags occupy disjoint bits, so addition combines them.
const WAIT_FOR_EXIT: libc::c_int = libc::WEXITED + libc::WNOWAIT;
/// Checks for the leader's exit without reaping it or blocking.
const POLL_FOR_EXIT: libc::c_int = WAIT_FOR_EXIT + libc::WNOHANG;

#[allow(unsafe_code)]
pub(super) fn exited(child: &mut Child, blocking: bool) -> io::Result<bool> {
    let flags = if blocking {
        WAIT_FOR_EXIT
    } else {
        POLL_FOR_EXIT
    };
    loop {
        // SAFETY: siginfo_t can be zeroed. waitid leaves si_signo zero when there is no exit.
        let mut status = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        // SAFETY: status points to writable storage, and P_PID selects our direct child.
        if unsafe { libc::waitid(libc::P_PID, child.id() as _, &raw mut status, flags) } == 0 {
            return Ok(status.si_signo != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

pub(super) struct ProcessGroup(Child);

impl ProcessGroup {
    pub(super) fn stop(&mut self, _child: &mut Child) -> io::Result<()> {
        // Waiting closes the sole write end of the watchdog's stdin.
        self.0.wait().map(|_| ())
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = self.0.wait();
    }
}

pub(super) fn spawn(command: &Command) -> io::Result<(Child, ProcessGroup)> {
    // UnrealIRCd calls setsid even in foreground mode. It must lead its group
    // so setsid cannot move it out of the group the watchdog will signal.
    let mut leader = Command::new("sh");
    leader
        .args(["-c", r#"read -r _ && exec "$0" "$@" </dev/null"#])
        .arg(command.get_program())
        .args(command.get_args())
        .process_group(0)
        .stdin(Stdio::piped());
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            leader.env(key, value);
        } else {
            leader.env_remove(key);
        }
    }
    if let Some(directory) = command.get_current_dir() {
        leader.current_dir(directory);
    }

    let mut child = super::spawn_with_stderr(&mut leader)?;
    // EOF before the watchdog joins prevents the server from starting.
    let mut gate = child.stdin.take().expect("piped server launch gate");
    let guarded = watchdog(&child).and_then(|group| {
        gate.write_all(b"start\n")?;
        Ok(group)
    });
    drop(gate);
    match guarded {
        Ok(group) => Ok((child, group)),
        Err(error) => {
            let _ = child.wait();
            Err(error)
        }
    }
}

fn watchdog(child: &Child) -> io::Result<ProcessGroup> {
    // The watchdog anchors the group even after the server exits. The owner's
    // write end of this stdin pipe is close-on-exec.
    Command::new("sh")
        .args([
            "-c",
            r#"trap '' TERM
               read -r _ || :
               kill -s TERM -- "-$0"
               sleep 0.1
               kill -s KILL -- "-$0""#,
        ])
        .arg(child.id().to_string())
        .process_group(child.id().cast_signed())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(ProcessGroup)
}

#[cfg(test)]
mod tests {
    mod group;

    use std::{process::Command, sync::mpsc, thread, time::Duration};

    use super::exited;

    #[test]
    fn observing_a_reaped_child_reports_the_wait_error() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();

        let (send, recv) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(exited(&mut child, false));
        });
        let error = recv
            .recv_timeout(Duration::from_secs(10))
            .expect("wait retried a permanent error")
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    }
}
