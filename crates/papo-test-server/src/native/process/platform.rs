use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub(super) fn prepare(command: &mut Command) {
    command.process_group(0);
}

#[allow(unsafe_code)]
pub(super) fn exited(child: &mut Child, blocking: bool) -> io::Result<bool> {
    let flags = libc::WEXITED | libc::WNOWAIT | if blocking { 0 } else { libc::WNOHANG };
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

#[allow(unsafe_code)]
pub(super) fn kill(child: &mut Child) -> io::Result<()> {
    let group = child.id().cast_signed();
    // SAFETY: prepare made the child the leader of its own group. Keeping it unreaped
    // reserves the group's identifier until after we signal it.
    if unsafe { libc::killpg(group, libc::SIGKILL) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{exited, kill};

    #[test]
    fn observing_a_reaped_child_reports_the_wait_error() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();

        let error = exited(&mut child, false).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn signaling_a_missing_group_succeeds() {
        // The unreaped child reserves its PID and inherits the test's group.
        let mut child = Command::new("true").spawn().unwrap();

        kill(&mut child).unwrap();
        assert!(child.wait().unwrap().success());
    }
}
