use papo::Event;
use papo_conformance_macros::scenario;

use crate::Scenario;

async fn connected(events: &mut papo::Events) {
    while !matches!(next_active_event(events).await, Event::Connected(_)) {}
}

async fn next_active_event(events: &mut papo::Events) -> Event {
    match events.next().await {
        Some(Event::Disconnected(disconnected)) => {
            panic!("disconnected while waiting for an event: {disconnected}")
        }
        Some(event) => event,
        None => panic!("driver stopped while waiting for an event"),
    }
}

async fn next_disconnect(events: &mut papo::Events) -> papo::Disconnected {
    loop {
        match events.next().await {
            Some(Event::Disconnected(disconnected)) => return disconnected,
            Some(_) => {}
            None => panic!("driver stopped before reporting a disconnection"),
        }
    }
}

/// keeps ordinary server traffic responsive without cloning the complete state
#[scenario(covers = ["security.h3.state-clone-bound"])]
async fn ordinary_traffic_avoids_state_snapshots(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    for index in 0..512 {
        wire.server(format!(
            ":user{index:03}!~user@users.example PRIVMSG papo-wire :audit message {index}"
        ))
        .await;
        while !matches!(next_active_event(&mut events).await, Event::Message(_)) {}
    }

    let ping = tokio::spawn(async move { client.ping().await });
    let sent = wire.next_sent().await;
    let token = sent.strip_prefix("PING ").expect("expected ping fence");
    wire.server(format!(":offline.test PONG offline.test :{token}"))
        .await;
    ping.await.unwrap()?;
    Ok(())
}

/// bounds accumulated ISUPPORT tokens while retaining known updates
#[scenario(covers = ["security.h4.isupport-bound"])]
async fn hostile_isupport_stays_bounded(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    for page in 0..22 {
        let tokens = (0..50)
            .map(|offset| format!("VENDOR{:04}=value", page * 50 + offset))
            .collect::<Vec<_>>()
            .join(" ");
        wire.server(format!(
            ":offline.test 005 papo-wire {tokens} :are supported"
        ))
        .await;
    }
    wire.server(":offline.test 005 papo-wire NICKLEN=31 :are supported")
        .await;

    let ping = tokio::spawn({
        let client = client.clone();
        async move { client.ping().await }
    });
    let sent = wire.next_sent().await;
    let token = sent.strip_prefix("PING ").expect("expected ping fence");
    wire.server(format!(":offline.test PONG offline.test :{token}"))
        .await;
    ping.await.unwrap()?;
    assert!(client.state().isupport().tokens().len() <= 1_024);
    assert_eq!(client.state().isupport().nicklen(), Some(31));
    Ok(())
}

/// bounds advertised capabilities and still requests a wanted late capability
#[scenario(covers = ["security.h5.capability-bound"])]
async fn hostile_cap_new_stays_bounded(_s: Scenario) -> papo_testing::Result<()> {
    let (_client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .request_caps(["vendor.example/late"])
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["cap-notify"]).await;
    connected(&mut events).await;
    for page in 0..12 {
        let caps = (0..50)
            .map(|offset| format!("vendor.example/cap{:04}=value", page * 50 + offset))
            .collect::<Vec<_>>()
            .join(" ");
        wire.server(format!(":offline.test CAP papo-wire NEW :{caps}"))
            .await;
    }
    wire.server(":offline.test CAP papo-wire NEW :vendor.example/late")
        .await;
    assert_eq!(wire.next_sent().await, "CAP REQ :vendor.example/late");
    Ok(())
}

/// abandons nested batches before retained tag data reaches 64 MiB
#[scenario(covers = ["security.h6.batch-byte-bound"])]
async fn hostile_batch_tags_hit_the_global_budget(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["batch", "labeled-response"]).await;
    connected(&mut events).await;
    let mut request =
        Box::pin(client.request_raw(papo::Line::new("VENDOR").with_params(["audit"])));
    tokio::select! {
        biased;
        result = &mut request => panic!("request finished before its response: {result:?}"),
        () = std::future::ready(()) => {}
    }
    assert_eq!(wire.next_sent().await, "@label=0 VENDOR audit");
    wire.server("@label=0 BATCH +b0 labeled-response").await;
    for depth in 1..64 {
        wire.server(format!("@batch=b{} BATCH +b{depth} vendor/sync", depth - 1))
            .await;
    }
    let member = format!(
        "@batch=b63;vendor.example/large={} :alice!~alice@users.example PRIVMSG #papo :payload",
        "x".repeat(8_000)
    );
    for _ in 0..140 {
        wire.server(&member).await;
    }

    loop {
        if matches!(
            next_active_event(&mut events).await,
            Event::Warning(papo::event::Warning::BatchAbandoned {
                reason: papo::event::BatchAbandonReason::ByteLimit,
                ..
            })
        ) {
            break;
        }
    }
    Ok(())
}

/// synchronizes a joined channel only on the first end-of-NAMES reply
#[scenario(covers = ["security.h8.single-channel-sync"])]
async fn duplicate_end_of_names_stays_inert(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    wire.server(":papo-wire!~papo@localhost JOIN #papo").await;
    wire.server(":offline.test 366 papo-wire #papo :End of NAMES list")
        .await;
    assert_eq!(wire.next_sent().await, "MODE #papo");
    assert_eq!(wire.next_sent().await, "WHO #papo");
    for _ in 0..3 {
        wire.server(":offline.test 366 papo-wire #papo :End of NAMES list")
            .await;
    }

    let ping = tokio::spawn(async move { client.ping().await });
    let sent = wire.next_sent().await;
    let token = sent
        .strip_prefix("PING ")
        .expect("duplicate 366 sent output");
    wire.server(format!(":offline.test PONG offline.test :{token}"))
        .await;
    ping.await.unwrap()?;
    Ok(())
}

/// ignores MONITOR presence for nicknames outside the desired set
#[scenario(covers = ["security.h11.monitored-presence-only"])]
async fn unsolicited_monitor_presence_stays_inert(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["extended-monitor"]).await;
    connected(&mut events).await;
    wire.server(":offline.test 005 papo-wire MONITOR=100 :are supported")
        .await;
    wire.server(":offline.test 730 papo-wire :mallory!~mallory@users.example")
        .await;
    let ping = tokio::spawn({
        let client = client.clone();
        async move { client.ping().await }
    });
    let sent = wire.next_sent().await;
    let token = sent.strip_prefix("PING ").expect("expected ping fence");
    wire.server(format!(":offline.test PONG offline.test :{token}"))
        .await;
    ping.await.unwrap()?;
    assert_eq!(client.state().monitor_online().count(), 0);

    let add = tokio::spawn({
        let client = client.clone();
        async move { client.monitor().add(["alice"]).await }
    });
    assert_eq!(wire.next_sent().await, "MONITOR + alice");
    add.await.unwrap()?;
    wire.server(":offline.test 730 papo-wire :alice!~alice@cloak.example")
        .await;
    while !matches!(
        next_active_event(&mut events).await,
        Event::MonitorOnline(_)
    ) {}
    assert_eq!(client.state().monitor_online().count(), 1);
    Ok(())
}

/// queues every autojoin after the correlated-request limit is full
#[scenario(covers = ["security.h7.rejoin-pending-bound"])]
async fn large_autojoin_set_reaches_the_wire(_s: Scenario) -> papo_testing::Result<()> {
    let channels = (0..300).map(|index| format!("#audit{index:03}"));
    let (_client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .autojoin(channels)
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["labeled-response"]).await;
    connected(&mut events).await;
    for index in 0..300 {
        let sent = wire.next_sent().await;
        let join = sent
            .strip_prefix("JOIN ")
            .or_else(|| sent.split_once(" JOIN ").map(|(_, join)| join))
            .unwrap_or_else(|| panic!("expected JOIN, got {sent}"));
        assert_eq!(join, format!("#audit{index:03}"));
        assert_eq!(sent.starts_with("@label="), index < 256);
    }
    Ok(())
}

/// restores a runtime away message only after the server confirms it
#[scenario(covers = ["security.d7.away-reconnect"])]
async fn confirmed_runtime_away_survives_reconnect(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    let away = tokio::spawn({
        let client = client.clone();
        async move { client.set_away("reviewing audit logs").await }
    });
    assert_eq!(wire.next_sent().await, "AWAY :reviewing audit logs");
    wire.server(":offline.test 306 papo-wire :You have been marked as being away")
        .await;
    away.await.unwrap()?;

    client.reconnect().await?;
    _ = next_disconnect(&mut events).await;
    wire.close().await;
    wire.register(&[]).await;
    connected(&mut events).await;
    assert_eq!(wire.next_sent().await, "AWAY :reviewing audit logs");
    Ok(())
}

/// stops reconnect catch-up when a non-final page repeats without progress
#[scenario(covers = ["security.h12.catchup-progress"])]
async fn repeated_catchup_page_stops_after_two_requests(_s: Scenario) -> papo_testing::Result<()> {
    let caps = [
        "batch",
        "draft/chathistory",
        "labeled-response",
        "server-time",
    ];
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .autojoin(["#papo"])
            .history_on_reconnect(true)
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&caps).await;
    connected(&mut events).await;
    assert_eq!(wire.next_sent().await, "@label=0 JOIN #papo");
    wire.server("@label=0 :papo-wire!~papo@localhost JOIN #papo")
        .await;
    wire.server("@time=2026-10-15T12:00:00.000Z;msgid=before-1 :alice!~alice@users.example PRIVMSG #papo :before disconnect")
        .await;
    while !matches!(next_active_event(&mut events).await, Event::Message(_)) {}

    client.reconnect().await?;
    _ = next_disconnect(&mut events).await;
    wire.close().await;
    wire.register(&caps).await;
    connected(&mut events).await;
    assert_eq!(wire.next_sent().await, "@label=1 JOIN #papo");
    assert_eq!(
        wire.next_sent().await,
        "@label=2 CHATHISTORY AFTER #papo msgid=before-1 100"
    );
    for line in [
        "@label=2 :offline.test BATCH +reply1 labeled-response",
        "@batch=reply1 :offline.test BATCH +history1 chathistory #papo",
        "@batch=history1;time=2026-10-15T12:01:00.000Z;msgid=missed-1 :alice!~alice@users.example PRIVMSG #papo :missed once",
        "@batch=reply1 :offline.test BATCH -history1",
        ":offline.test BATCH -reply1",
    ] {
        wire.server(line).await;
    }
    assert_eq!(
        wire.next_sent().await,
        "@label=3 CHATHISTORY AFTER #papo msgid=missed-1 100"
    );
    for line in [
        "@label=3 :offline.test BATCH +reply2 labeled-response",
        "@batch=reply2 :offline.test BATCH +history2 chathistory #papo",
        "@batch=history2;time=2026-10-15T12:01:00.000Z;msgid=missed-1 :alice!~alice@users.example PRIVMSG #papo :missed once",
        "@batch=reply2 :offline.test BATCH -history2",
        ":offline.test BATCH -reply2",
    ] {
        wire.server(line).await;
    }
    loop {
        if let Event::History(history) = next_active_event(&mut events).await {
            assert_eq!(history.messages().count(), 1);
            break;
        }
    }

    let ping = tokio::spawn(async move { client.ping().await });
    let sent = wire.next_sent().await;
    assert!(sent.starts_with("PING ") || sent.contains(" PING "));
    ping.abort();
    Ok(())
}
