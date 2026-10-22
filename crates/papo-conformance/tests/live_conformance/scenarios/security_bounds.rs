use papo::Event;
use papo_conformance_macros::scenario;

use crate::Scenario;

async fn connected(events: &mut papo::Events) {
    while !matches!(events.next().await, Some(Event::Connected(_))) {}
}

/// keeps ordinary server traffic responsive without cloning the complete state
#[scenario(covers = ["security.h3.state-clone-bound"])]
async fn ordinary_traffic_avoids_state_snapshots(_s: Scenario) -> papo_testing::Result<()> {
    let (client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&[]).await;
    connected(&mut events).await;
    for index in 0..512 {
        wire.server(format!(
            ":user{index:03}!~user@users.example PRIVMSG papo-wire :audit message {index}"
        ))
        .await;
        while !matches!(events.next().await, Some(Event::Message(_))) {}
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
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
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
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
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
    let (_client, mut events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .rate_limit(papo::RateLimit::unlimited()),
    );
    wire.register(&["batch"]).await;
    connected(&mut events).await;
    wire.server("@label=lost BATCH +b0 labeled-response").await;
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
            events.next().await,
            Some(Event::Warning(papo::event::Warning::BatchAbandoned {
                reason: papo::event::BatchAbandonReason::ByteLimit,
                ..
            }))
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
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
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
        papo::Client::builder("irc://offline.test")
            .nick("papo-wire")
            .request_caps(["extended-monitor"])
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
    while !matches!(events.next().await, Some(Event::MonitorOnline(_))) {}
    assert_eq!(client.state().monitor_online().count(), 1);
    Ok(())
}
