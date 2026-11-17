use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

/// Registers `nick`, sends `after_welcome` once the server sends 001, and returns every line up to
/// the first one `last` accepts, answering PING along the way.
pub(crate) fn lines_until(
    address: SocketAddr,
    nick: &str,
    after_welcome: &[u8],
    last: impl Fn(&str) -> bool,
) -> Vec<String> {
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut connection = BufReader::new(stream);
    let register = format!("NICK {nick}\r\nUSER papo 0 * :papo\r\n");
    connection.get_mut().write_all(register.as_bytes()).unwrap();
    let mut lines = Vec::new();
    while !lines.last().is_some_and(|line: &String| last(line)) {
        let mut line = String::new();
        assert_ne!(connection.read_line(&mut line).unwrap(), 0, "{lines:?}");
        if let Some(token) = line.strip_prefix("PING ") {
            let pong = format!("PONG {token}");
            connection.get_mut().write_all(pong.as_bytes()).unwrap();
        } else if line.split(' ').nth(1) == Some("001") {
            connection.get_mut().write_all(after_welcome).unwrap();
        }
        lines.push(line.trim_end().to_owned());
    }
    lines
}
