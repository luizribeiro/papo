use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
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
    client_bytes: Arc<Mutex<Vec<u8>>>,
    server_bytes: Arc<Mutex<Vec<u8>>>,
}
impl FaultProxy {
    pub(crate) fn start(upstream: SocketAddr) -> io::Result<Self> {
        Self::start_with_cap_delay(upstream, false)
    }

    pub(crate) fn start_with_hostname_notice(upstream: SocketAddr) -> io::Result<Self> {
        Self::start_with_cap_delay(upstream, true)
    }

    fn start_with_cap_delay(upstream: SocketAddr, delay_cap: bool) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let (commands, receiver) = mpsc::channel();
        let client_bytes = Arc::new(Mutex::new(Vec::new()));
        let server_bytes = Arc::new(Mutex::new(Vec::new()));
        let recorded_client = Arc::clone(&client_bytes);
        let recorded_server = Arc::clone(&server_bytes);
        let worker = thread::spawn(move || {
            forward(
                &listener,
                upstream,
                &receiver,
                &recorded_client,
                &recorded_server,
                delay_cap,
            );
        });
        Ok(Self {
            address,
            commands,
            worker: Some(worker),
            client_bytes,
            server_bytes,
        })
    }
    /// Returns the endpoint that accepts one client connection.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }
    /// Returns all bytes forwarded in each direction so far.
    #[must_use]
    pub fn wire_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        let recover = std::sync::PoisonError::into_inner;
        (
            self.client_bytes.lock().unwrap_or_else(recover).clone(),
            self.server_bytes.lock().unwrap_or_else(recover).clone(),
        )
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

fn forward(
    listener: &TcpListener,
    upstream: SocketAddr,
    commands: &mpsc::Receiver<Command>,
    client_bytes: &Mutex<Vec<u8>>,
    server_bytes: &Arc<Mutex<Vec<u8>>>,
    delay_cap: bool,
) {
    loop {
        let Ok((mut client, _)) = listener.accept() else {
            return;
        };
        let Ok(mut server) = TcpStream::connect(upstream) else {
            return;
        };
        let (Ok(mut client_writer), Ok(mut server_reader)) =
            (client.try_clone(), server.try_clone())
        else {
            return;
        };
        let recorded_server = Arc::clone(server_bytes);
        let (notice_sender, notice_receiver) = mpsc::sync_channel(1);
        let reverse = thread::spawn(move || {
            forward_server(
                &mut server_reader,
                &mut client_writer,
                &recorded_server,
                &notice_sender,
            );
        });
        let _ = client.set_read_timeout(Some(Duration::from_millis(20)));
        let mut buffer = [0_u8; 4096];
        let mut held_cap = Vec::new();
        let mut stop = false;
        'connection: loop {
            if !held_cap.is_empty() && notice_receiver.try_recv().is_ok() {
                if server.write_all(&held_cap).is_err() {
                    break;
                }
                held_cap.clear();
            }
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
                            Ok(Command::Close) | Err(_) => {
                                stop = true;
                                break;
                            }
                            Ok(Command::BlackHole(acknowledge)) => {
                                let _ = acknowledge.send(());
                            }
                        }
                    }
                    break;
                }
                Ok(Command::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                    stop = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            match client.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    client_bytes
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .extend_from_slice(&buffer[..count]);
                    if forward_client_bytes(&mut server, &buffer[..count], delay_cap, &mut held_cap)
                        .is_err()
                    {
                        break 'connection;
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
        if stop {
            return;
        }
    }
}

fn forward_server(
    server: &mut TcpStream,
    client: &mut TcpStream,
    recorded: &Mutex<Vec<u8>>,
    notice: &mpsc::SyncSender<()>,
) {
    let mut buffer = [0_u8; 4096];
    while let Ok(count) = server.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let mut recorded = recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recorded.extend_from_slice(&buffer[..count]);
        if recorded
            .windows(b"NOTICE * :*** Looking up your hostname...".len())
            .any(|window| window == b"NOTICE * :*** Looking up your hostname...")
        {
            _ = notice.try_send(());
        }
        drop(recorded);
        if client.write_all(&buffer[..count]).is_err() {
            break;
        }
    }
}

fn forward_client_bytes(
    server: &mut TcpStream,
    bytes: &[u8],
    delay_cap: bool,
    held: &mut Vec<u8>,
) -> io::Result<()> {
    if !delay_cap {
        return server.write_all(bytes);
    }
    if held.is_empty() && bytes.starts_with(b"CAP LS 302\r\n") {
        let cap_end = b"CAP LS 302\r\n".len();
        let user_start = bytes[cap_end..]
            .windows(b"USER ".len())
            .position(|window| window == b"USER ")
            .map_or(bytes.len(), |offset| cap_end + offset);
        held.extend_from_slice(&bytes[..cap_end]);
        held.extend_from_slice(&bytes[user_start..]);
        return server.write_all(&bytes[cap_end..user_start]);
    }
    if held.is_empty() {
        return server.write_all(bytes);
    }
    let Some(user_start) = bytes
        .windows(b"USER ".len())
        .position(|window| window == b"USER ")
    else {
        return server.write_all(bytes);
    };
    held.extend_from_slice(&bytes[user_start..]);
    server.write_all(&bytes[..user_start])
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use super::FaultProxy;

    #[test]
    fn forwards_bytes_then_drops_exactly_its_connection() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = upstream.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut connection, _) = upstream.accept().unwrap();
                let mut request = [0; 4];
                connection.read_exact(&mut request).unwrap();
                assert_eq!(&request, b"PING");
                connection.write_all(b"PONG").unwrap();
                let mut eof = [0];
                assert_eq!(connection.read(&mut eof).unwrap(), 0);
            }
        });
        let proxy = FaultProxy::start(address).unwrap();
        let mut client = TcpStream::connect(proxy.address()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client.write_all(b"PING").unwrap();
        let mut reply = [0; 4];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"PONG");
        assert_eq!(proxy.wire_bytes(), (b"PING".to_vec(), b"PONG".to_vec()));
        proxy.drop_connection().unwrap();
        assert_eq!(client.read(&mut reply).unwrap_or(0), 0);
        let mut replacement = TcpStream::connect(proxy.address()).unwrap();
        replacement.write_all(b"PING").unwrap();
        replacement.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"PONG");
        drop(replacement);
        server.join().unwrap();
    }

    #[test]
    fn black_hole_stops_reading_client_bytes() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = upstream.local_addr().unwrap();
        let (accepted, connection) = mpsc::sync_channel(0);
        let server = thread::spawn(move || {
            let (stream, _) = upstream.accept().unwrap();
            accepted.send(stream).unwrap();
        });
        let proxy = FaultProxy::start(address).unwrap();
        let mut client = TcpStream::connect(proxy.address()).unwrap();
        let mut upstream = connection.recv_timeout(Duration::from_secs(1)).unwrap();
        proxy.black_hole().unwrap();
        client.write_all(b"QUIT :bounded shutdown\r\n").unwrap();
        upstream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut byte = [0];
        assert!(upstream.read(&mut byte).is_err());
        drop(proxy);
        server.join().unwrap();
    }
}
