//! Catchup output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{process::Command, time::Duration};

use papo::{Client, Endpoint, Event, Line, Sasl};
use papo_test_server::{create_account, from_env, unique};
use tokio::time::timeout;

const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

#[tokio::test(flavor = "current_thread")]
async fn prints_three_unread_messages_and_advances_the_marker() {
    timeout(Duration::from_secs(20), async {
        let server = from_env();
        let endpoint = Endpoint::plaintext(
            server.plaintext_addr().ip().to_string(),
            server.plaintext_addr().port(),
        )
        .unwrap();
        let channel = format!("#{}", unique("papo-catchup"));
        let account = unique("papo-catchup");
        let poster_nick = unique("papo-poster");
        create_account(&server, &account, PASSWORD).unwrap();

        let (persistent, mut persistent_events) = Client::builder(endpoint.clone(), &account)
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(&account, papo::Secret::new(PASSWORD)))
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut persistent_events, &channel).await;
        persistent
            .send_raw(Line::new("PERSISTENCE").with_params(["SET", "ON"]))
            .await
            .unwrap();
        persistent.quit().await.unwrap();

        let (poster, mut poster_events) = Client::builder(endpoint.clone(), &poster_nick)
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut poster_events, &channel).await;
        let texts = [
            "the deployment started",
            "the migration completed",
            "the service is healthy",
        ];
        let mut newest = None;
        for text in texts {
            let sent = poster.privmsg(&channel, text).await.unwrap();
            let [
                papo::SentLine {
                    confirmation: papo::Confirmation::Confirmed { server_time, .. },
                    ..
                },
            ] = sent.lines.as_slice()
            else {
                panic!("Ergo should confirm each history seed as one line")
            };
            newest = *server_time;
        }

        let output = catchup_command(&server, &account, &channel)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            texts.map(|text| format!("{channel} {text}"))
        );

        let (reader, mut reader_events) = Client::builder(endpoint, &account)
            .allow_insecure_auth(true)
            .sasl(Sasl::plain(&account, papo::Secret::new(PASSWORD)))
            .autojoin([&channel])
            .connect()
            .await
            .unwrap();
        wait_synced(&mut reader_events, &channel).await;
        assert!(matches!(
            reader.fetch_read_marker(&channel).await.unwrap(),
            papo::ReadMarker::ReadUntil(stored) if Some(stored) >= newest
        ));
        reader.quit().await.unwrap();
        poster.quit().await.unwrap();
    })
    .await
    .expect("live catchup test did not finish within 20 seconds");
}

#[tokio::test(flavor = "current_thread")]
async fn waits_for_channel_sync_before_catching_up() {
    let server = from_env();
    let channel = format!("#{}", unique("papo-invite-only"));
    let account = unique("papo-catchup");
    create_account(&server, &account, PASSWORD).unwrap();
    let (observer, mut events) = timeout(Duration::from_secs(20), async {
        let (observer, mut events) = Client::builder(
            format!("irc://{}", server.plaintext_addr()),
            unique("papo-observer"),
        )
        .autojoin([&channel])
        .connect()
        .await
        .unwrap();
        wait_synced(&mut events, &channel).await;
        observer.set_modes(&channel).add('i').await.unwrap();
        (observer, events)
    })
    .await
    .expect("could not prepare the invite-only channel");
    let _watch = observer.watch(&account);

    let mut child = catchup_command(&server, &account, &channel)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let online = timeout(Duration::from_secs(10), async {
        while let Some((event, _)) = events.next().await {
            if matches!(event, Event::MonitorOnline(online) if online.subject.nick() == account.as_str())
            {
                return true;
            }
        }
        false
    })
    .await;
    let early_exit = timeout(Duration::from_secs(3), async {
        while child.try_wait()?.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, std::io::Error>(())
    })
    .await;
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();

    assert!(matches!(online, Ok(true)), "example never came online");
    assert!(
        early_exit.is_err(),
        "example exited before channel sync: {output:?}"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    observer.quit().await.unwrap();
}

fn catchup_command(server: &papo_test_server::ServerInfo, account: &str, channel: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_catchup"));
    command
        .arg(format!("ircs://{}", server.tls_addr()))
        .env("SSL_CERT_FILE", server.ca_pem_path())
        .env("IRC_NICK", account)
        .env("IRC_PASSWORD", PASSWORD)
        .env("IRC_CHANNEL", channel);
    command
}

async fn wait_synced(events: &mut papo::Events, channel: &str) {
    loop {
        match events.next().await {
            Some((
                Event::ChannelSynced(papo::event::ChannelSynced {
                    channel: synced, ..
                }),
                _,
            )) if synced.name() == channel => return,
            Some((Event::Disconnected(disconnected), _)) => {
                panic!("disconnected: {disconnected}")
            }
            Some(_) => {}
            None => panic!("driver stopped while joining"),
        }
    }
}
