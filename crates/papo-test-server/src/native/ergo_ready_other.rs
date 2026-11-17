use std::{io, net::SocketAddr, path::Path, process::Command};

use super::{process::ServerChild, server::wait_for_child_with, startup::StartupBudget};

pub(crate) struct ReadySignal;

impl ReadySignal {
    pub(crate) fn bind(_directory: &Path, _command: &mut Command) -> io::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn wait(
        self,
        addresses: [SocketAddr; 4],
        budget: StartupBudget,
        child: &mut ServerChild,
    ) -> io::Result<()> {
        wait_for_child_with("Ergo", addresses, budget, child)
    }
}
