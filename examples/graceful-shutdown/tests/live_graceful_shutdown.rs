//! Graceful process shutdown against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{
    future::Future,
    io::Read as _,
    net::SocketAddr,
    process::{Child, Stdio},
    time::Duration,
};

use papo::{Client, Event, Events, Watch};
use papo_test_server::{ServerInfo, from_env, unique};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};

#[tokio::test(flavor = "current_thread")]
async fn sigterm_sends_quit_and_exits_successfully() {
    within_deadline(async {
        let mut watched = Watched::start().await;
        let child = watched.spawn_example(&watched.server_url());
        watched.wait_for_online().await;
        terminate(&child);
        watched.expect_graceful_exit(child).await;
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn sigterm_while_connecting_still_sends_quit() {
    within_deadline(async {
        let mut watched = Watched::start().await;
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let child = watched.spawn_example(&plaintext_url(relay.local_addr().unwrap()));
        let (example_side, _) = relay.accept().await.unwrap();
        terminate(&child);
        let server_side = TcpStream::connect(watched.server.plaintext_addr())
            .await
            .unwrap();
        let (sent, ()) = tokio::join!(relay_recording_sent(example_side, server_side), async {
            watched.wait_for_online().await;
            watched.expect_graceful_exit(child).await;
        });
        assert!(sent.contains("\r\nQUIT :shutting down\r\n"), "{sent}");
    })
    .await;
}

struct Watched {
    server: ServerInfo,
    nick: String,
    observer: Client,
    events: Events,
    _watch: Watch,
}

impl Watched {
    async fn start() -> Self {
        let server = from_env();
        let nick = unique("papo-shutdown");
        let (observer, events) = Client::builder(
            plaintext_url(server.plaintext_addr()),
            unique("papo-observer"),
        )
        .connect()
        .await
        .unwrap();
        let watch = observer.query(&nick).unwrap().watch();
        Self {
            server,
            nick,
            observer,
            events,
            _watch: watch,
        }
    }

    fn server_url(&self) -> String {
        plaintext_url(self.server.plaintext_addr())
    }

    fn spawn_example(&self, url: &str) -> Child {
        std::process::Command::new(env!("CARGO_BIN_EXE_graceful-shutdown"))
            .arg(url)
            .env("IRC_NICK", &self.nick)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    async fn wait_for_online(&mut self) {
        let nick = self.nick.clone();
        wait_for(&mut self.events, |event| {
            matches!(event, Event::MonitorOnline(online) if online.subject.nick() == nick.as_str())
        })
        .await;
    }

    async fn expect_graceful_exit(&mut self, mut child: Child) {
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            sleep(Duration::from_millis(10)).await;
        };
        assert!(status.success(), "{status}");
        let nick = self.nick.clone();
        wait_for(&mut self.events, |event| {
            matches!(event, Event::MonitorOffline(offline) if offline.subject == nick.as_str())
        })
        .await;
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert_eq!(stderr, "shutting down\n");
        self.observer.quit().await.unwrap();
    }
}

fn plaintext_url(address: SocketAddr) -> String {
    format!("irc://{address}")
}

async fn within_deadline(test: impl Future<Output = ()>) {
    timeout(Duration::from_secs(20), test)
        .await
        .expect("graceful shutdown test did not finish within 20 seconds");
}

fn terminate(child: &Child) {
    let signal = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
}

async fn relay_recording_sent(example: TcpStream, server: TcpStream) -> String {
    let (mut from_example, mut to_example) = example.into_split();
    let (mut from_server, mut to_server) = server.into_split();
    let upstream = async {
        let mut sent = Vec::new();
        let mut chunk = [0; 4096];
        while let Ok(read @ 1..) = from_example.read(&mut chunk).await {
            sent.extend_from_slice(&chunk[..read]);
            if to_server.write_all(&chunk[..read]).await.is_err() {
                break;
            }
        }
        let _ = to_server.shutdown().await;
        String::from_utf8(sent).unwrap()
    };
    let downstream = async move {
        let _ = tokio::io::copy(&mut from_server, &mut to_example).await;
    };
    tokio::join!(upstream, downstream).0
}

async fn wait_for(events: &mut Events, mut wanted: impl FnMut(&Event) -> bool) {
    while let Some((event, _)) = events.next().await {
        if wanted(&event) {
            return;
        }
    }
    panic!("driver stopped before the expected event");
}
