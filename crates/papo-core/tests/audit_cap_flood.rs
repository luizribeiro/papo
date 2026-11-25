//! Transcript coverage for capability requests the server never answers.

#![allow(unsafe_code)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

use papo_core::{Config, RateLimit, Session};

struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Every operation is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: The original layout is forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: The original pointer and layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn drain(session: &mut Session) -> usize {
    while session.poll_event().is_some() {}
    std::iter::from_fn(|| session.poll_transmit()).count()
}

#[test]
fn unanswered_capability_requests_stay_bounded() {
    let now = Instant::now();
    let mut config = Config::default()
        .nick("papo")
        .wanted_caps(["multi-prefix", "away-notify"]);
    config.rate_limit = RateLimit::unlimited();
    let mut session = Session::new(config);
    session.handle_connected(now);
    for line in [
        ":ergo.test CAP * LS :multi-prefix",
        ":ergo.test CAP papo ACK :multi-prefix",
        ":ergo.test 001 papo :Welcome to the ErgoTest IRC Network papo",
        ":ergo.test 005 papo CHANTYPES=# NETWORK=ErgoTest :are supported by this server",
        ":ergo.test 422 papo :MOTD File is missing",
    ] {
        session.handle_line(now, line.as_bytes());
    }
    drain(&mut session);

    let duplicate = b":ergo.test CAP papo NEW :away-notify";
    session.handle_line(now, duplicate);
    assert_eq!(drain(&mut session), 1);

    let feed = |session: &mut Session, bursts: usize| {
        for _ in 0..bursts {
            for _ in 0..1_000 {
                session.handle_line(now, duplicate);
            }
            assert_eq!(drain(session), 0, "CAP REQ is already pending");
        }
        LIVE_BYTES.load(Ordering::Relaxed)
    };
    let warmed = feed(&mut session, 2);
    let flooded = feed(&mut session, 20);

    let grown = flooded.saturating_sub(warmed);
    assert!(
        grown < 64 * 1024,
        "20000 more CAP NEW lines retained {grown} more bytes"
    );
}
