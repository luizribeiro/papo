//! Persistent reconnects against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]
#![cfg(feature = "unstable")]

mod support;

use support::within;

use std::{net::SocketAddr, time::Duration};

use papo::{Client, Endpoint, Event, PersistenceSetting, Reconnect, Sasl};
use papo_test_server::create_account;
use papo_test_server::{from_env, unique};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn persistent_server_restores_channels_without_a_client_join() {
    within(async {
        let server = from_env();
        let (proxy, disconnect, mut client_lines) =
            disconnecting_proxy(server.plaintext_addr()).await;
        let address = Endpoint::plaintext(proxy.ip().to_string(), proxy.port()).unwrap();
        let account = unique("papo-persistent");
        let channel = format!("#{}", unique("papo-persistent"));
        create_account(&server, &account, PASSWORD).unwrap();
        let (client, mut events) = Client::builder(address)
            .nick(support::nick(&account))
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(&account, PASSWORD))
            .request_caps(["sasl", "draft/persistence", "labeled-response"])
            .enable(papo::Feature::Persistence)
            .autojoin([&channel])
            .reconnect(Reconnect::exponential().max_delay(Duration::from_secs(1)))
            .connect()
            .await
            .unwrap();
        wait_for(
            &mut events,
            |event| matches!(event, Event::ChannelSynced(papo::event::ChannelSynced { channel: joined, .. }) if joined == channel.as_str()),
        )
        .await;
        while client_lines.try_recv().is_ok() {}
        let persistence = client
            .persistence()
            .set(PersistenceSetting::On)
            .await
            .unwrap();
        assert!(persistence.effective);

        disconnect.send(()).await.unwrap();
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Disconnected(disconnected) if disconnected.is_reconnecting()
            )
        })
        .await;
        wait_for(
            &mut events,
            |event| matches!(event, Event::Connected(connected) if connected.reconnected),
        )
        .await;
        wait_for(&mut events, |event| {
            matches!(
                event,
                Event::Joined(papo::event::Joined { channel: joined, .. })
                    | Event::ChannelSynced(papo::event::ChannelSynced { channel: joined, .. })
                    if joined == channel.as_str()
            )
        })
        .await;
        assert!(client.state().channel(&channel).is_some());
        client.ping().await.unwrap();
        assert!(
            std::iter::from_fn(|| client_lines.try_recv().ok())
                .all(|line| line != format!("JOIN {channel}")),
            "papo sent a duplicate JOIN after the server restored the channel"
        );
        client
            .quit("persistent reconnect verified")
            .await
            .unwrap();
    })
    .await;
}

async fn disconnecting_proxy(
    target: SocketAddr,
) -> (
    SocketAddr,
    mpsc::Sender<()>,
    mpsc::UnboundedReceiver<String>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (disconnect, mut disconnects) = mpsc::channel(1);
    let (client_lines, lines) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();
            let server = TcpStream::connect(target).await.unwrap();
            let (client_read, mut client_write) = client.into_split();
            let (mut server_read, server_write) = server.into_split();
            tokio::select! {
                _ = forward_client_lines(client_read, server_write, client_lines.clone()) => {}
                _ = tokio::io::copy(&mut server_read, &mut client_write) => {}
                _ = disconnects.recv() => {}
            }
        }
    });
    (address, disconnect, lines)
}

async fn forward_client_lines(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    lines: mpsc::UnboundedSender<String>,
) -> std::io::Result<()> {
    let mut buffered = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            return Ok(());
        }
        writer.write_all(&chunk[..count]).await?;
        buffered.extend_from_slice(&chunk[..count]);
        while let Some(end) = buffered.iter().position(|byte| *byte == b'\n') {
            let line = buffered.drain(..=end).collect::<Vec<_>>();
            _ = lines.send(String::from_utf8_lossy(&line).trim().to_owned());
        }
    }
}

async fn wait_for(events: &mut papo::Events, accepted: impl Fn(&Event) -> bool) {
    loop {
        let event = events.next().await.expect("driver stopped unexpectedly");
        if accepted(&event) {
            return;
        }
    }
}
