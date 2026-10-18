use std::{
    io::{self, Read},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};
enum Command {
    Drop(mpsc::SyncSender<()>),
    BlackHole(mpsc::SyncSender<()>),
    Close,
}
/// A dedicated TCP forwarding endpoint with deterministic fault controls.
pub struct FaultProxy {
    address: SocketAddr,
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}
impl FaultProxy {
    pub(crate) fn start(upstream: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let (commands, receiver) = mpsc::channel();
        let worker = thread::spawn(move || forward(&listener, upstream, &receiver));
        Ok(Self {
            address,
            commands,
            worker: Some(worker),
        })
    }
    /// Returns the endpoint that accepts one client connection.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }
    /// Abruptly closes both sides of the proxied connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the proxy worker has already stopped.
    pub fn drop_connection(&self) -> io::Result<()> {
        self.control(Command::Drop)
    }
    /// Stops reading bytes sent by the client while leaving its socket open.
    ///
    /// # Errors
    ///
    /// Returns an error if the proxy worker has already stopped.
    pub fn black_hole(&self) -> io::Result<()> {
        self.control(Command::BlackHole)
    }

    fn control(&self, command: impl FnOnce(mpsc::SyncSender<()>) -> Command) -> io::Result<()> {
        let (acknowledge, acknowledged) = mpsc::sync_channel(0);
        self.commands
            .send(command(acknowledge))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "fault proxy stopped"))?;
        acknowledged
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "fault proxy stopped"))
    }
}

fn forward(listener: &TcpListener, upstream: SocketAddr, commands: &mpsc::Receiver<Command>) {
    let Ok((mut client, _)) = listener.accept() else {
        return;
    };
    let Ok(mut server) = TcpStream::connect(upstream) else {
        return;
    };
    let (Ok(mut client_writer), Ok(mut server_reader)) = (client.try_clone(), server.try_clone())
    else {
        return;
    };
    let reverse = thread::spawn(move || {
        let _ = io::copy(&mut server_reader, &mut client_writer);
    });
    let _ = client.set_read_timeout(Some(Duration::from_millis(20)));
    let mut buffer = [0_u8; 4096];
    'forward: loop {
        match commands.try_recv() {
            Ok(Command::Drop(acknowledge)) => {
                let _ = client.shutdown(Shutdown::Both);
                let _ = server.shutdown(Shutdown::Both);
                let _ = acknowledge.send(());
                break;
            }
            Ok(Command::BlackHole(acknowledge)) => {
                let _ = acknowledge.send(());
                loop {
                    match commands.recv() {
                        Ok(Command::Drop(acknowledge)) => {
                            let _ = acknowledge.send(());
                            break;
                        }
                        Ok(Command::Close) | Err(_) => break,
                        Ok(Command::BlackHole(acknowledge)) => {
                            let _ = acknowledge.send(());
                        }
                    }
                }
                break;
            }
            Ok(Command::Close) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match client.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if std::io::Write::write_all(&mut server, &buffer[..count]).is_err() {
                    break 'forward;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    let _ = client.shutdown(Shutdown::Both);
    let _ = server.shutdown(Shutdown::Both);
    let _ = reverse.join();
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
        if let Some(worker) = self.worker.take() {
            let _ = TcpStream::connect(self.address);
            let _ = worker.join();
        }
    }
}
