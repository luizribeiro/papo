use std::{
    fs, io,
    net::SocketAddr,
    os::unix::net::UnixDatagram,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use super::process::ServerChild;

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
        deadline: Instant,
        child: &mut ServerChild,
    ) -> io::Result<()> {
        let mut message = [0; 512];
        while Instant::now() < deadline {
            child.check("Ergo")?;
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
        Err(child.timeout("Ergo"))
    }
}
