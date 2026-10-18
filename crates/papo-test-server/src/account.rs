use std::{
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use crate::{DeploymentProfile, ServerInfo};

/// Creates an account through `IRCv3` account registration.
///
/// # Errors
///
/// Returns an error when the credentials contain IRC line delimiters, the
/// connection fails, or the selected server does not accept the account.
pub fn register_account(server: &ServerInfo, name: &str, password: &str) -> io::Result<()> {
    if !credentials_valid(name, password) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "account credentials contain IRC line delimiters",
        ));
    }
    match DeploymentProfile::from_env()? {
        DeploymentProfile::ErgoDefault | DeploymentProfile::ErgoPersistent => {
            register_account_at(server.plaintext_addr(), name, password)
        }
        DeploymentProfile::InspIrcd
        | DeploymentProfile::Solanum
        | DeploymentProfile::UnrealIrcd => register_atheme(server.plaintext_addr(), name, password),
        DeploymentProfile::SojuErgo => register_account_at(
            server
                .bouncer()
                .ok_or_else(|| io::Error::other("soju profile lacks bouncer metadata"))?
                .upstream_addr(),
            name,
            password,
        ),
    }
}

/// Creates an account through the IRC listener at `address`.
///
/// # Errors
///
/// Returns an error when the credentials contain IRC line delimiters, the
/// connection fails, or Ergo does not accept the account.
pub fn register_account_at(address: SocketAddr, name: &str, password: &str) -> io::Result<()> {
    if !credentials_valid(name, password) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "account credentials contain IRC line delimiters",
        ));
    }
    let stream = TcpStream::connect(address)?;
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

pub(crate) fn register_atheme(address: SocketAddr, name: &str, password: &str) -> io::Result<()> {
    if password.contains(' ') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Atheme passwords cannot contain spaces",
        ));
    }
    let stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut connection = BufReader::new(stream);
    send(
        &mut connection,
        &format!("NICK {name}\r\nUSER papo 0 * :papo account registration\r\n"),
    )?;
    read_until(&mut connection, |line| line.contains(" 001 "))?;
    send(
        &mut connection,
        &format!("PRIVMSG NickServ :REGISTER {password} papo@example.invalid\r\n"),
    )?;
    for _ in 0..128 {
        let mut line = String::new();
        if connection.read_line(&mut line)? == 0 {
            break;
        }
        if line.contains(" is now registered to ") {
            return Ok(());
        }
        if line.contains(" is already registered.") {
            return Err(io::Error::other("Atheme rejected account registration"));
        }
    }
    Err(io::Error::other(
        "Atheme did not complete account registration",
    ))
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
                "server closed during account registration",
            ));
        }
        if let Some(token) = line.strip_prefix("PING ") {
            send(connection, &format!("PONG {token}"))?;
            continue;
        }
        if line.contains(" FAIL REGISTER ") {
            return Err(io::Error::other(line.trim().to_owned()));
        }
        if accepted(&line) {
            return Ok(());
        }
    }
    Err(io::Error::other(
        "server did not complete account registration",
    ))
}

#[cfg(test)]
mod tests {
    use std::{io, net::SocketAddr};

    use super::{credentials_valid, register_atheme};

    #[test]
    fn rejects_every_irc_line_delimiter() {
        assert!(credentials_valid("papo-account", "q7Hx-2mZp_9vLcT4wRfB"));
        assert!(!credentials_valid("papo account", "valid-password"));
        assert!(!credentials_valid("papo\raccount", "valid-password"));
        assert!(!credentials_valid("papo\naccount", "valid-password"));
        assert!(!credentials_valid("papo-account", "invalid\rpassword"));
        assert!(!credentials_valid("papo-account", "invalid\npassword"));
        let address = "127.0.0.1:1".parse::<SocketAddr>().unwrap();
        assert_eq!(
            register_atheme(address, "papo-account", "invalid password")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
