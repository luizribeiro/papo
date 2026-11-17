use std::time::Duration;
use std::{io, net::SocketAddr, path::Path, process::Command, time::Instant};

use super::{process::ServerChild, server::wait_for_child_until};

pub(crate) struct ReadySignal;

impl ReadySignal {
    pub(crate) fn bind(_directory: &Path, _command: &mut Command) -> io::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn wait(
        self,
        addresses: [SocketAddr; 4],
        child: &mut ServerChild,
    ) -> io::Result<()> {
        wait_for_child_until(
            "Ergo",
            addresses,
            Instant::now() + Duration::from_secs(10),
            child,
        )
    }
}
