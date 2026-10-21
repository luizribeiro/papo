use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::mpsc::UnboundedReceiver,
    task::JoinHandle,
};

const MAX_LINE_LEN: usize = 8191 + 512;

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
        self.expect_sent("CAP LS 302").await;
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
            self.expect_sent("CAP END").await;
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
    /// Panics if `line` contains a line ending, exceeds 8703 bytes, or the
    /// client has closed its end of the pipe.
    pub async fn server(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        assert!(
            !line.contains(['\r', '\n']),
            "server input must be one line"
        );
        assert!(line.len() <= MAX_LINE_LEN, "server line exceeds 8703 bytes");
        let mut bytes = line.as_bytes().to_vec();
        bytes.extend_from_slice(b"\r\n");
        papo::__testing::server_line(self.stream().await, &bytes).await;
    }

    /// Reads the next complete line sent by the client, without its terminator.
    ///
    /// # Panics
    ///
    /// Panics if the client closes the pipe mid-line, sends non-UTF-8 text, or
    /// sends a line longer than 8703 bytes.
    pub async fn next_sent(&mut self) -> String {
        let stream = self.stream().await;
        let mut line = Vec::new();
        loop {
            let byte = stream.read_u8().await.expect("client closed its stream");
            line.push(byte);
            assert!(line.len() <= MAX_LINE_LEN, "client line exceeds 8703 bytes");
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

    async fn expect_sent(&mut self, expected: &str) {
        let mut expected = expected.as_bytes().to_vec();
        expected.extend_from_slice(b"\r\n");
        papo::__testing::client_line(self.stream().await, &expected).await;
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::wire;

    #[tokio::test(start_paused = true)]
    async fn debug_identifies_the_fixture_without_exposing_its_state() {
        let secret = "unique-wire-password-7462";
        let (_client, _events, wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .password(secret),
        );

        let debug = format!("{wire:?}");
        assert_eq!(debug, "Wire { .. }");
        assert!(!debug.contains(secret));
    }

    async fn next_connected(events: &mut papo::Events) -> papo::event::Connected {
        loop {
            match events.next().await {
                Some(papo::Event::Connected(connected)) => return connected,
                Some(_) => {}
                None => panic!("driver stopped before connecting"),
            }
        }
    }

    async fn next_disconnected(events: &mut papo::Events) -> papo::Disconnected {
        loop {
            match events.next().await {
                Some(papo::Event::Disconnected(disconnected)) => return disconnected,
                Some(_) => {}
                None => panic!("driver stopped before disconnecting"),
            }
        }
    }

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
        _ = next_connected(&mut events).await;
        wire.server("PING :ergo.example/12345").await;
        assert_eq!(wire.next_sent().await, "PONG :ergo.example/12345");
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_wire_stops_the_client_driver() {
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
        _ = next_connected(&mut events).await;

        drop(wire);
        assert!(events.next().await.is_none());
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

    #[tokio::test(start_paused = true)]
    async fn reconnects_after_the_pipe_closes() {
        let (_client, mut events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .rate_limit(papo::RateLimit::unlimited())
                .reconnect(
                    papo::Reconnect::exponential()
                        .max_delay(Duration::from_secs(1))
                        .max_attempts(2),
                ),
        );
        wire.register(&[]).await;
        assert!(!next_connected(&mut events).await.reconnected);

        wire.close().await;
        let delay = next_disconnected(&mut events)
            .await
            .reconnect_in
            .expect("disconnect should retry");
        tokio::time::advance(delay).await;
        wire.register(&[]).await;
        assert!(next_connected(&mut events).await.reconnected);
    }

    #[tokio::test(start_paused = true)]
    async fn request_timeout_uses_the_paused_clock() {
        let (client, _events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .rate_limit(papo::RateLimit::unlimited())
                .request_timeout(Duration::from_secs(10))
                .reconnect(papo::Reconnect::never()),
        );
        wire.register(&[]).await;
        let request = tokio::spawn({
            let nick = client.state().nickname("alice");
            async move { client.whois(nick).await }
        });
        assert_eq!(wire.next_sent().await, "WHOIS alice");

        tokio::time::advance(Duration::from_secs(9)).await;
        assert!(!request.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(matches!(request.await.unwrap(), Err(papo::Error::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn accepts_the_literal_server_line_bound() {
        let (_client, _events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .reconnect(papo::Reconnect::never()),
        );
        wire.server("x".repeat(8191 + 512)).await;
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "server line exceeds 8703 bytes")]
    async fn rejects_one_byte_beyond_the_literal_server_line_bound() {
        let (_client, _events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .reconnect(papo::Reconnect::never()),
        );
        wire.server("x".repeat(8191 + 512 + 1)).await;
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "server input must be one line")]
    async fn rejects_embedded_server_line_endings() {
        let (_client, _events, mut wire) = wire(
            papo::Client::builder("irc://offline.test")
                .nick("papo-wire")
                .reconnect(papo::Reconnect::never()),
        );
        wire.server("PING :one\r\nPING :two").await;
    }
}
