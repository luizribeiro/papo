use std::io;
use std::process::{Child, Command};

pub(super) fn prepare(_command: &mut Command) {}

pub(super) fn exited(child: &mut Child, blocking: bool) -> io::Result<bool> {
    if blocking {
        child.wait()?;
        return Ok(true);
    }
    Ok(child.try_wait()?.is_some())
}

pub(super) struct ProcessGroup;

impl ProcessGroup {
    pub(super) fn of(_child: &Child) -> Self {
        Self
    }

    pub(super) fn stop(&self, child: &mut Child) -> io::Result<()> {
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        Ok(())
    }
}
