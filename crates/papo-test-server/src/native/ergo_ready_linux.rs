use std::{
    fs, io, net::SocketAddr, os::unix::net::UnixDatagram, path::Path, process::Command,
    time::Duration,
};

use super::{process::ServerChild, startup::StartupBudget};

pub(crate) struct ReadySignal {
    socket: UnixDatagram,
}

impl ReadySignal {
    pub(crate) fn bind(directory: &Path, command: &mut Command) -> io::Result<Self> {
        let path = directory.join("notify.sock");
        if let Err(error) = fs::remove_file(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(error);
        }
        let socket = UnixDatagram::bind(&path)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        command.env("NOTIFY_SOCKET", path);
        Ok(Self { socket })
    }

    pub(crate) fn wait(
        self,
        _addresses: [SocketAddr; 4],
        mut budget: StartupBudget,
        child: &mut ServerChild,
    ) -> io::Result<()> {
        let mut message = [0; 512];
        loop {
            child.check("Ergo")?;
            child.charge("Ergo", &mut budget)?;
            match self.socket.recv(&mut message) {
                Ok(count)
                    if message[..count]
                        .split(|byte| *byte == b'\n')
                        .any(|line| line == b"READY=1") =>
                {
                    return Ok(());
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, net::SocketAddr, process::Command};

    use super::ReadySignal;
    use crate::native::{
        directory::TemporaryDirectory, process::ServerChild, startup::tests::settled_budget,
    };

    #[test]
    fn a_silent_idle_child_spends_the_readiness_budget() {
        let directory = TemporaryDirectory::create().unwrap();
        let mut command = Command::new("sleep");
        command.arg("60");
        let ready = ReadySignal::bind(directory.path(), &mut command).unwrap();
        let mut child = ServerChild::spawn(&mut command).unwrap();
        let budget = settled_budget(&[child.id()], false);

        let error = ready
            .wait(
                [SocketAddr::from(([127, 0, 0, 1], 6667)); 4],
                budget,
                &mut child,
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            error
                .to_string()
                .starts_with("Ergo did not start: stayed idle before becoming ready (11.")
        );
    }
}
