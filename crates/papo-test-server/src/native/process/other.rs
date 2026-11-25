use std::io;
use std::process::{Child, Command};

pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, ProcessGroup)> {
    super::spawn_with_stderr(command).map(|child| (child, ProcessGroup))
}

pub(super) fn exited(child: &mut Child, blocking: bool) -> io::Result<bool> {
    if blocking {
        child.wait()?;
        return Ok(true);
    }
    Ok(child.try_wait()?.is_some())
}

pub(super) struct ProcessGroup;

impl ProcessGroup {
    pub(super) fn stop(&mut self, child: &mut Child) -> io::Result<()> {
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        Ok(())
    }
}
