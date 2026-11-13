//! Differential coverage for user handles with optional API compiled in.
#![cfg(not(browser))]

use papo::{Client, Event, Events, RateLimit, Reconnect};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream},
    task::JoinHandle,
};

const CAPS: &str =
    "account-notify away-notify chghost extended-join message-tags multi-prefix server-time";

const WELCOME: &[&str] = &[
    ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
    ":ergo.test 002 papo :Your host is ergo.test, running version ergo-2.16.0",
    ":ergo.test 003 papo :This server was created Wed Oct 15 2025 at 12:00:00 UTC",
    ":ergo.test 004 papo ergo.test ergo-2.16.0 BERTZios CEIMRUabefhiklmnoqstuv Iabefhkloqv",
    ":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# MONITOR=100 NETWORK=ErgoTest PREFIX=(qaohv)~&@%+ UTF8ONLY WHOX :are supported by this server",
    ":ergo.test 376 papo :End of /MOTD command",
    ":papo!~papo@127.0.0.1 JOIN #release * :papo",
    ":ergo.test 353 papo = #release :@papo",
    ":ergo.test 366 papo #release :End of /NAMES list",
    ":alice!~alice@cloak.example JOIN #release alice :Alice Example",
    "@time=2026-10-15T12:00:00.000Z;msgid=ErgoMessage42 :alice!~alice@cloak.example PRIVMSG #release :release candidate is ready",
];

struct Server {
    stream: BufReader<DuplexStream>,
}

impl Server {
    async fn send(&mut self, line: &str) {
        println!("SERVER {line}");
        let stream = self.stream.get_mut();
        stream.write_all(line.as_bytes()).await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
    }

    async fn sent(&mut self) -> Option<String> {
        let mut line = String::new();
        if self.stream.read_line(&mut line).await.unwrap() == 0 {
            return None;
        }
        let line = line.trim_end().to_owned();
        println!("CLIENT {line}");
        Some(line)
    }

    async fn expect(&mut self, prefix: &str) -> String {
        loop {
            let line = self.sent().await.expect("client closed its stream");
            if line.starts_with(prefix) {
                return line;
            }
        }
    }

    async fn register(&mut self) {
        self.expect("CAP LS 302").await;
        self.expect("NICK papo").await;
        self.expect("USER ").await;
        self.send(&format!(":ergo.test CAP * LS :{CAPS}")).await;
        let requested = self.expect("CAP REQ :").await;
        let requested = requested.strip_prefix("CAP REQ :").unwrap().to_owned();
        self.send(&format!(":ergo.test CAP papo ACK :{requested}"))
            .await;
        self.expect("CAP END").await;
        for line in WELCOME {
            self.send(line).await;
        }
    }
}

fn print_event(event: &Event) {
    let mut text = format!("{event:?}");
    while let Some(start) = text.find("received_at: SystemTime {") {
        let end = start + text[start..].find('}').unwrap() + 1;
        text.replace_range(start..end, "received_at: <local clock>");
    }
    #[cfg(feature = "unstable")]
    let text = text.replace(", relayed_by: None", "");
    println!("EVENT {text}");
}

async fn until(events: &mut Events, mut done: impl FnMut(&Event) -> bool) -> Event {
    loop {
        let (event, _) = events.next().await.expect("driver stopped");
        print_event(&event);
        if done(&event) {
            return event;
        }
    }
}

async fn finish<T: std::fmt::Debug>(name: &str, request: JoinHandle<T>) -> T {
    let result = request.await.unwrap();
    println!("RESULT {name} {result:?}");
    result
}

#[tokio::test]
async fn stable_and_unstable_replay_identically() {
    let builder = Client::builder("irc://ergo.test", "papo")
        .rate_limit(RateLimit::unlimited())
        .reconnect(Reconnect::never());
    let (client, mut events, driver, mut connections) = papo::__testing::wire_client(builder);
    let driver = tokio::spawn(driver);
    let (stream, _) = connections.recv().await.unwrap();
    let mut server = Server {
        stream: BufReader::new(stream),
    };

    println!("PAPO_HANDLE_PARITY_BEGIN");
    server.register().await;
    let Event::Message(request) = until(&mut events, |event| event.as_message().is_some()).await
    else {
        unreachable!()
    };
    let reply = tokio::spawn(
        client
            .privmsg(&request.actor, "approved for release")
            .into_future(),
    );
    server.expect("PRIVMSG alice :approved for release").await;
    finish("reply", reply).await.unwrap();

    let peer = client.query("alice").unwrap();
    let watch = peer.watch();
    server.expect("MONITOR + alice").await;
    server
        .send(":ergo.test 730 papo :alice!~alice@cloak.example")
        .await;
    until(&mut events, |event| event.as_monitor_online().is_some()).await;
    println!("WATCH live={}", watch.is_live());

    let quit = tokio::spawn(client.quit("release shipped").into_future());
    server.expect("QUIT :release shipped").await;
    drop(server);
    finish("quit", quit).await.unwrap();
    while let Some((event, _)) = events.next().await {
        print_event(&event);
    }
    println!("PAPO_HANDLE_PARITY_END");
    driver.await.unwrap().unwrap();
}
