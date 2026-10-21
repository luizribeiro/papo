use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::mpsc::UnboundedReceiver,
    task::JoinHandle,
};

const MAX_LINE_LEN: usize = 8191;

/// The server side of an in-memory IRC connection.
pub struct Wire {
    stream: Option<DuplexStream>,
    connections: UnboundedReceiver<DuplexStream>,
    driver: JoinHandle<Result<(), papo::Error>>,
}

impl std::fmt::Debug for Wire {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Wire").finish_non_exhaustive()
    }
}

/// Builds a real client connected to an in-memory IRC server.
///
/// Call this from a Tokio runtime. The client driver starts immediately and
/// reconnects through a fresh in-memory stream according to `builder`'s policy.
#[must_use]
pub fn wire(builder: papo::Builder) -> (papo::Client, papo::Events, Wire) {
    let (client, events, driver, connections) = papo::__testing::wire_client(builder);
    let driver = tokio::spawn(driver);
    (
        client,
        events,
        Wire {
            stream: None,
            connections,
            driver,
        },
    )
}

impl Wire {
    /// Completes connection setup with a representative Ergo server burst.
    ///
    /// `caps` are advertised in `CAP LS`. Capabilities requested by the client
    /// are acknowledged before numerics 001 through 005 and a short MOTD.
    ///
    /// # Panics
    ///
    /// Panics if the client does not send a valid default connection burst or
    /// stops during capability negotiation.
    pub async fn register(&mut self, caps: &[&str]) {
        assert_eq!(self.next_sent().await, "CAP LS 302");
        let nick_line = self.next_sent().await;
        let nick = nick_line
            .strip_prefix("NICK ")
            .expect("client did not send NICK");
        assert!(
            self.next_sent().await.starts_with("USER "),
            "client did not send USER"
        );

        self.server(format!(":offline.test CAP * LS :{}", caps.join(" ")))
            .await;
        let negotiation = self.next_sent().await;
        if let Some(requested) = negotiation.strip_prefix("CAP REQ :") {
            self.server(format!(":offline.test CAP * ACK :{requested}"))
                .await;
            assert_eq!(self.next_sent().await, "CAP END");
        } else {
            assert_eq!(negotiation, "CAP END");
        }

        self.server(format!(
            ":offline.test 001 {nick} :Welcome to the ErgoTest IRC Network"
        ))
        .await;
        self.server(format!(
            ":offline.test 002 {nick} :Your host is offline.test, running version ergo-2.16.0"
        ))
        .await;
        self.server(format!(
            ":offline.test 003 {nick} :This server was created Wed Oct 15 2025 at 12:00:00 UTC"
        ))
        .await;
        self.server(format!(
            ":offline.test 004 {nick} offline.test ergo-2.16.0 BioZ beI,k,l,imnst"
        ))
        .await;
        self.server(format!(
            ":offline.test 005 {nick} CASEMAPPING=rfc1459 CHANTYPES=# PREFIX=(qaohv)~&@%+ UTF8ONLY :are supported by this server"
        ))
        .await;
        self.server(format!(
            ":offline.test 375 {nick} :- offline.test Message of the day -"
        ))
        .await;
        self.server(format!(
            ":offline.test 372 {nick} :- Welcome to the Ergo test network"
        ))
        .await;
        self.server(format!(":offline.test 376 {nick} :End of /MOTD command"))
            .await;
    }

    /// Sends one server line to the client.
    ///
    /// # Panics
    ///
    /// Panics if `line` contains a line ending, exceeds 8191 bytes, or the
    /// client has closed its end of the pipe.
    pub async fn server(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        assert!(
            !line.contains(['\r', '\n']),
            "server input must be one line"
        );
        assert!(line.len() <= MAX_LINE_LEN, "server line exceeds 8191 bytes");
        let stream = self.stream().await;
        stream.write_all(line.as_bytes()).await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
    }

    /// Reads the next complete line sent by the client, without its terminator.
    ///
    /// # Panics
    ///
    /// Panics if the client closes the pipe mid-line, sends non-UTF-8 text, or
    /// sends a line longer than 8191 bytes.
    pub async fn next_sent(&mut self) -> String {
        let stream = self.stream().await;
        let mut line = Vec::new();
        loop {
            let byte = stream.read_u8().await.expect("client closed its stream");
            line.push(byte);
            assert!(line.len() <= MAX_LINE_LEN, "client line exceeds 8191 bytes");
            if line.ends_with(b"\r\n") {
                line.truncate(line.len() - 2);
                return String::from_utf8(line).expect("client sent non-UTF-8 text");
            }
        }
    }

    /// Closes the current server connection.
    ///
    /// # Panics
    ///
    /// Panics if the in-memory pipe cannot be shut down.
    pub async fn close(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            stream.shutdown().await.unwrap();
        }
    }

    async fn stream(&mut self) -> &mut DuplexStream {
        if self.stream.is_none() {
            self.stream = Some(
                self.connections
                    .recv()
                    .await
                    .expect("client driver stopped before connecting"),
            );
        }
        self.stream.as_mut().unwrap()
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::wire;

    #[tokio::test(start_paused = true)]
    async fn exchanges_complete_irc_lines() {
        let (_client, mut events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .reconnect(papo::Reconnect::never()),
        );
        assert_eq!(wire.next_sent().await, "CAP LS 302");
        assert_eq!(wire.next_sent().await, "NICK papo-wire");
        assert_eq!(wire.next_sent().await, "USER papo-wire 0 * papo-wire");
        wire.server(":offline.test CAP * LS :").await;
        assert_eq!(wire.next_sent().await, "CAP END");
        wire.server(":offline.test 001 papo-wire :Welcome to ErgoTest")
            .await;
        assert!(matches!(
            events.next().await,
            Some(papo::Event::Connected(_))
        ));
        wire.server("PING :ergo.example/12345").await;
        assert_eq!(wire.next_sent().await, "PONG :ergo.example/12345");
    }

    #[tokio::test(start_paused = true)]
    async fn bot_answers_ping_entirely_offline() {
        let (client, mut events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .reconnect(papo::Reconnect::never()),
        );
        wire.register(&["message-tags"]).await;
        wire.server(
            "@time=2026-10-21T10:00:00.000Z :alice!~alice@cloak.example PRIVMSG papo-wire :!ping",
        )
        .await;

        while let Some(event) = events.next().await {
            let Some(message) = event.as_message() else {
                continue;
            };
            if message.text == "!ping" {
                client
                    .privmsg(message.return_address(), "pong")
                    .await
                    .unwrap();
                break;
            }
        }
        assert_eq!(wire.next_sent().await, "PRIVMSG alice pong");
    }
}
