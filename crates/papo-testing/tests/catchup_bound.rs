//! A reconnect catchup the server never answers must not keep growing with live traffic.

#![allow(unsafe_code)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use papo::{Events, Reconnect};
use papo_testing::{Wire, wire};

struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Every operation is forwarded unchanged to the system allocator; only the byte count
// of live allocations is recorded.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE_BYTES.fetch_add(layout.size(), Ordering::SeqCst);
        // SAFETY: The original allocation layout is forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::SeqCst);
        // SAFETY: The original pointer and layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const CAPS: [&str; 5] = [
    "message-tags",
    "server-time",
    "batch",
    "labeled-response",
    "draft/chathistory",
];

async fn join_papo(wire: &mut Wire, label: &str) {
    assert_eq!(wire.next_sent().await, format!("@label={label} JOIN #papo"));
    for line in [
        format!("@label={label} :ergo.test BATCH +join{label} labeled-response"),
        format!("@batch=join{label} :papo!~u@3t4fgnf9zitka.irc JOIN #papo"),
        format!("@batch=join{label} :ergo.test 353 papo = #papo :papo @alice"),
        format!("@batch=join{label} :ergo.test 366 papo #papo :End of /NAMES list"),
        format!(":ergo.test BATCH -join{label}"),
    ] {
        wire.server(line).await;
    }
}

async fn drain_events(events: &mut Events) {
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(5), events.next()).await {}
}

async fn flood(wire: &mut Wire, events: &mut Events, round: usize) {
    for batch in 0..20 {
        for index in 0..50 {
            wire.server(format!(
                "@time=2026-11-16T09:{round:02}:{batch:02}.{index:03}Z;msgid=r{round:02}b{batch:02}i{index:02}x95ke2uf27w64gb :mallory!~m@cloak.example PRIVMSG #papo :deploy log line {index}"
            ))
            .await;
        }
        drain_events(events).await;
    }
}

#[tokio::test(start_paused = true)]
async fn unanswered_catchup_never_grows_with_live_traffic() {
    let (_client, mut events, mut wire) = wire(
        papo::Client::builder("irc://ergo.test", "papo")
            .rate_limit(papo::RateLimit::unlimited())
            .autojoin(["#papo"])
            .history_on_reconnect(true)
            .request_timeout(Duration::from_secs(10))
            .reconnect(Reconnect::exponential().max_delay(Duration::from_secs(1))),
    );
    wire.register(&CAPS).await;
    join_papo(&mut wire, "0").await;
    wire.server("@time=2026-11-16T09:00:00.000Z;msgid=RfcBTzyxY120004 :alice!~a@cloak.example PRIVMSG #papo :deploy finished").await;
    drain_events(&mut events).await;

    wire.close().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    wire.register(&CAPS).await;
    join_papo(&mut wire, "1").await;
    assert_eq!(
        wire.next_sent().await,
        "@label=2 CHATHISTORY AFTER #papo msgid=RfcBTzyxY120004 100"
    );
    tokio::time::advance(Duration::from_secs(11)).await;
    drain_events(&mut events).await;

    flood(&mut wire, &mut events, 0).await;
    let settled = LIVE_BYTES.load(Ordering::SeqCst);
    for round in 1..4 {
        flood(&mut wire, &mut events, round).await;
    }
    let growth = LIVE_BYTES.load(Ordering::SeqCst).saturating_sub(settled);
    assert!(growth < 16 * 1024, "3,000 live lines kept {growth} bytes");
}
