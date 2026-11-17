use std::{
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use crate::{DeploymentProfile, ServerInfo};

/// Creates an account through `IRCv3` `draft/account-registration`.
///
/// # Errors
///
/// Returns an error when the credentials contain IRC line delimiters, the
/// connection fails, or the selected server does not accept the account.
pub fn create_account(server: &ServerInfo, name: &str, password: &str) -> io::Result<()> {
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
    read_until(&mut connection, "waiting for CAP LS", |line| {
        line.contains(" CAP * LS ") && line.contains("draft/account-registration")
    })?;
    send(&mut connection, "CAP REQ :draft/account-registration\r\n")?;
    read_until(&mut connection, "waiting for CAP ACK", |line| {
        line.contains(" CAP * ACK ") && line.contains("draft/account-registration")
    })?;
    send(&mut connection, &format!("NICK {name}\r\n"))?;
    send(
        &mut connection,
        &format!("REGISTER {name} * :{password}\r\n"),
    )?;
    read_until(&mut connection, "waiting for REGISTER SUCCESS", |line| {
        line.contains(" REGISTER SUCCESS ")
    })
}

/// Changes an Ergo account password through its local `NickServ` service.
///
/// # Errors
///
/// Returns an error for invalid credentials, connection failures, or a rejected change.
pub fn change_account_password_at(
    address: SocketAddr,
    name: &str,
    current: &str,
    replacement: &str,
) -> io::Result<()> {
    if !credentials_valid(name, current)
        || replacement.contains([' ', '\r', '\n'])
        || current.contains(' ')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "account credentials contain IRC parameter delimiters",
        ));
    }
    let stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut connection = BufReader::new(stream);
    send(
        &mut connection,
        &format!("PASS {name}:{current}\r\nNICK {name}\r\nUSER papo 0 * :papo password change\r\n"),
    )?;
    read_until(
        &mut connection,
        "waiting for the welcome after PASS",
        |line| line.contains(" 001 "),
    )?;
    send(
        &mut connection,
        &format!("PRIVMSG NickServ :PASSWD {current} {replacement} {replacement}\r\n"),
    )?;
    read_reply(
        &mut connection,
        "waiting for NickServ to answer PASSWD",
        passwd_verdict,
    )
}

fn passwd_verdict(line: &str) -> Option<io::Result<()>> {
    (line.contains("NickServ") && line.contains(" NOTICE ")).then(|| {
        if line.contains("Password changed") {
            Ok(())
        } else {
            Err(io::Error::other("NickServ rejected the password change"))
        }
    })
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
    read_until(&mut connection, "waiting for the welcome", |line| {
        line.contains(" 001 ")
    })?;
    send(
        &mut connection,
        &format!("PRIVMSG NickServ :REGISTER {password} papo@example.invalid\r\n"),
    )?;
    read_reply(
        &mut connection,
        "waiting for NickServ to answer REGISTER",
        atheme_verdict,
    )
}

fn atheme_verdict(line: &str) -> Option<io::Result<()>> {
    if line.contains(" is now registered to ") {
        Some(Ok(()))
    } else if line.contains(" is already registered.") {
        Some(Err(io::Error::other(
            "Atheme rejected account registration",
        )))
    } else {
        None
    }
}

fn credentials_valid(name: &str, password: &str) -> bool {
    !name.contains([' ', '\r', '\n']) && !password.contains(['\r', '\n'])
}

fn send(connection: &mut BufReader<TcpStream>, line: &str) -> io::Result<()> {
    connection.get_mut().write_all(line.as_bytes())
}

fn read_until(
    connection: &mut BufReader<TcpStream>,
    step: &str,
    accepted: impl Fn(&str) -> bool,
) -> io::Result<()> {
    read_reply(connection, step, |line| {
        if line.contains(" FAIL REGISTER ") {
            Some(Err(io::Error::other(line.trim().to_owned())))
        } else {
            accepted(line).then_some(Ok(()))
        }
    })
}

fn read_reply(
    connection: &mut BufReader<TcpStream>,
    step: &str,
    decide: impl Fn(&str) -> Option<io::Result<()>>,
) -> io::Result<()> {
    for _ in 0..128 {
        let line = read_line(connection, step)?;
        if let Some(token) = line.strip_prefix("PING ") {
            send(connection, &format!("PONG {token}"))?;
        } else if let Some(verdict) = decide(&line) {
            return verdict;
        }
    }
    Err(io::Error::other(format!(
        "server sent 128 lines without an answer while {step}"
    )))
}

fn read_line(connection: &mut BufReader<TcpStream>, step: &str) -> io::Result<String> {
    let mut line = String::new();
    match connection.read_line(&mut line) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("server closed the connection while {step}"),
        )),
        Ok(_) => Ok(line),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("{error} while {step}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, BufReader, Write},
        net::{Shutdown, SocketAddr, TcpListener, TcpStream},
        time::Duration,
    };

    use super::{atheme_verdict, credentials_valid, passwd_verdict, read_until, register_atheme};

    const ERGO_LOGIN: &str = ":ergo.test 900 * * papo-password :You are now logged in as papo-password\r\n\
                              :ergo.test 903 * :Authentication successful\r\n";

    fn scripted_server(lines: &str) -> (BufReader<TcpStream>, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server.write_all(lines.as_bytes()).unwrap();
        (BufReader::new(client), server)
    }

    fn wait_for_welcome(connection: &mut BufReader<TcpStream>) -> io::Error {
        read_until(connection, "waiting for the welcome after PASS", |line| {
            line.contains(" 001 ")
        })
        .unwrap_err()
    }

    #[test]
    fn a_read_timeout_names_the_stalled_step() {
        let (mut connection, _server) = scripted_server(ERGO_LOGIN);
        connection
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let error = wait_for_welcome(&mut connection);
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{error:?}"
        );
        assert!(
            error
                .to_string()
                .ends_with(" while waiting for the welcome after PASS"),
            "{error}"
        );
    }

    #[test]
    fn a_closed_connection_names_the_stalled_step() {
        let (mut connection, server) = scripted_server(ERGO_LOGIN);
        server.shutdown(Shutdown::Write).unwrap();
        let error = wait_for_welcome(&mut connection);
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(
            error.to_string(),
            "server closed the connection while waiting for the welcome after PASS"
        );
    }

    #[test]
    fn endless_unrelated_lines_name_the_stalled_step() {
        let (mut connection, _server) = scripted_server(
            &":ergo.test NOTICE * :*** Looking up your hostname...\r\n".repeat(128),
        );
        let error = wait_for_welcome(&mut connection);
        assert_eq!(
            error.to_string(),
            "server sent 128 lines without an answer while waiting for the welcome after PASS"
        );
    }

    #[test]
    fn answers_ping_while_waiting() {
        let (mut connection, mut server) = scripted_server(
            "PING papo-password\r\n\
             :ergo.test 001 papo-password :Welcome to the ErgoTest IRC Network papo-password\r\n",
        );
        read_until(&mut connection, "waiting for the welcome", |line| {
            line.contains(" 001 ")
        })
        .unwrap();
        server.shutdown(Shutdown::Write).unwrap();
        drop(connection);
        let mut answered = String::new();
        io::Read::read_to_string(&mut server, &mut answered).unwrap();
        assert_eq!(answered, "PONG papo-password\r\n");
    }

    #[test]
    fn reads_nickserv_passwd_answers_from_ergo() {
        let changed = ":NickServ!NickServ@localhost NOTICE papo-password :Password changed";
        let refused = ":NickServ!NickServ@localhost NOTICE papo-password :Password incorrect";
        let mode = ":ergo.test 221 papo-password +Zi";
        assert!(passwd_verdict(changed).unwrap().is_ok());
        assert!(passwd_verdict(refused).unwrap().is_err());
        assert!(passwd_verdict(mode).is_none());
    }

    #[test]
    fn reads_nickserv_register_answers_from_atheme() {
        let registered = ":nickserv!nickserv@services.int NOTICE papo-account \
                          :papo-account is now registered to papo@example.invalid.";
        let taken = ":nickserv!nickserv@services.int NOTICE papo-account \
                     :papo-account is already registered.";
        let reminder = ":nickserv!nickserv@services.int NOTICE papo-account \
                        :This nickname is registered. Please choose a different nickname, \
                        or identify via /msg nickserv IDENTIFY papo-account <password>";
        assert!(atheme_verdict(registered).unwrap().is_ok());
        assert!(atheme_verdict(taken).unwrap().is_err());
        assert!(atheme_verdict(reminder).is_none());
    }

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
