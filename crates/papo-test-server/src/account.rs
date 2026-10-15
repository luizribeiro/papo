use std::{
    io::{self, BufRead, BufReader, Write},
    net::TcpStream,
    time::Duration,
};

use crate::ServerInfo;

/// Creates an account through `IRCv3` account registration.
///
/// # Errors
///
/// Returns an error when the credentials contain IRC line delimiters, the
/// connection fails, or Ergo does not accept the account.
pub fn register_account(server: &ServerInfo, name: &str, password: &str) -> io::Result<()> {
    if !credentials_valid(name, password) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "account credentials contain IRC line delimiters",
        ));
    }
    let stream = TcpStream::connect(server.plaintext_addr())?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut connection = BufReader::new(stream);
    send(&mut connection, "CAP LS 302\r\n")?;
    read_until(&mut connection, |line| {
        line.contains(" CAP * LS ") && line.contains("draft/account-registration")
    })?;
    send(&mut connection, "CAP REQ :draft/account-registration\r\n")?;
    read_until(&mut connection, |line| {
        line.contains(" CAP * ACK ") && line.contains("draft/account-registration")
    })?;
    send(&mut connection, &format!("NICK {name}\r\n"))?;
    send(
        &mut connection,
        &format!("REGISTER {name} * :{password}\r\n"),
    )?;
    read_until(&mut connection, |line| line.contains(" REGISTER SUCCESS "))
}

fn credentials_valid(name: &str, password: &str) -> bool {
    !name.contains([' ', '\r', '\n']) && !password.contains(['\r', '\n'])
}

fn send(connection: &mut BufReader<TcpStream>, line: &str) -> io::Result<()> {
    connection.get_mut().write_all(line.as_bytes())
}

fn read_until(
    connection: &mut BufReader<TcpStream>,
    accepted: impl Fn(&str) -> bool,
) -> io::Result<()> {
    for _ in 0..128 {
        let mut line = String::new();
        if connection.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ergo closed during account registration",
            ));
        }
        if line.contains(" FAIL REGISTER ") {
            return Err(io::Error::other(line.trim().to_owned()));
        }
        if accepted(&line) {
            return Ok(());
        }
    }
    Err(io::Error::other(
        "Ergo did not complete account registration",
    ))
}

#[cfg(test)]
mod tests {
    use super::credentials_valid;

    #[test]
    fn rejects_every_irc_line_delimiter() {
        assert!(credentials_valid("papo-account", "q7Hx-2mZp_9vLcT4wRfB"));
        assert!(!credentials_valid("papo account", "valid-password"));
        assert!(!credentials_valid("papo\raccount", "valid-password"));
        assert!(!credentials_valid("papo\naccount", "valid-password"));
        assert!(!credentials_valid("papo-account", "invalid\rpassword"));
        assert!(!credentials_valid("papo-account", "invalid\npassword"));
    }
}
