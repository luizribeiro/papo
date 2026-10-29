//! Allocator-level regression coverage for queued and returned secret wire lines.

#![allow(unsafe_code)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

use papo_core::{Config, RateLimit, Sasl, Secret, Session};
use papo_proto::Line;

struct InspectingAllocator;

static TARGET: AtomicUsize = AtomicUsize::new(0);
static TARGET_LEN: AtomicUsize = AtomicUsize::new(0);
static WATCHED: AtomicUsize = AtomicUsize::new(0);
static WATCHED_LEN: AtomicUsize = AtomicUsize::new(0);
static LEAKED: AtomicBool = AtomicBool::new(false);
static WIPED: AtomicBool = AtomicBool::new(false);

// SAFETY: Every operation is forwarded to `System` with its original pointer and layout;
// inspection happens before forwarding deallocation.
unsafe impl GlobalAlloc for InspectingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The system allocator receives the unchanged allocation layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The allocation is still live until it is forwarded to `System` below.
        let bytes = unsafe { std::slice::from_raw_parts(pointer, layout.size()) };
        if pointer as usize == WATCHED.load(Ordering::SeqCst) {
            let length = WATCHED_LEN.load(Ordering::SeqCst);
            WIPED.store(
                bytes[..length].iter().all(|byte| *byte == 0),
                Ordering::SeqCst,
            );
            WATCHED.store(0, Ordering::SeqCst);
        }
        let target_pointer = TARGET.load(Ordering::SeqCst) as *const u8;
        let target_len = TARGET_LEN.load(Ordering::SeqCst);
        if !target_pointer.is_null() && target_len <= bytes.len() {
            // SAFETY: The target is a static byte string registered by this test.
            let target = unsafe { std::slice::from_raw_parts(target_pointer, target_len) };
            if bytes.windows(target_len).any(|window| window == target) {
                LEAKED.store(true, Ordering::SeqCst);
            }
        }
        // SAFETY: The pointer and layout are forwarded unchanged to their originating allocator.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: InspectingAllocator = InspectingAllocator;

fn inspect(marker: &'static str) {
    LEAKED.store(false, Ordering::SeqCst);
    TARGET_LEN.store(marker.len(), Ordering::SeqCst);
    TARGET.store(marker.as_ptr() as usize, Ordering::SeqCst);
}

#[test]
fn wipes_queued_and_polled_secret_wire_lines() {
    const QUEUED: &str = "queued-pass-marker-6184";
    const POLLED: &str = "polled-pass-marker-4729";
    const SASL: &str = "AGFsaWNlAHNhc2wtd2lwZS1tYXJrZXItNjkzOA==";

    inspect(QUEUED);
    let mut session = Session::new(
        Config::default()
            .allow_insecure_auth(true)
            .rate_limit(RateLimit::unlimited()),
    );
    session
        .send(Instant::now(), Line::new("PASS").param(Secret::new(QUEUED)))
        .unwrap();
    drop(session);
    assert!(!LEAKED.load(Ordering::SeqCst));

    inspect(POLLED);
    let mut session = Session::new(
        Config::default()
            .allow_insecure_auth(true)
            .rate_limit(RateLimit::unlimited()),
    );
    session
        .send(Instant::now(), Line::new("PASS").param(Secret::new(POLLED)))
        .unwrap();
    let line = session.poll_transmit().unwrap();
    WATCHED_LEN.store(line.len(), Ordering::SeqCst);
    WATCHED.store(line.as_ptr() as usize, Ordering::SeqCst);
    drop(line);
    assert!(WIPED.load(Ordering::SeqCst));
    assert!(!LEAKED.load(Ordering::SeqCst));

    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .sasl(Sasl::plain(
                "alice",
                papo_core::Secret::new("sasl-wipe-marker-6938"),
            ))
            .allow_insecure_auth(true)
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    while session.poll_transmit().is_some() {}
    session.handle_line(now, b"CAP * LS :sasl=PLAIN");
    while session.poll_transmit().is_some() {}
    session.handle_line(now, b"CAP * ACK :sasl");
    while session.poll_transmit().is_some() {}
    inspect(SASL);
    session.handle_line(now, b"AUTHENTICATE +");
    let line = session.poll_transmit().unwrap();
    assert!(
        line.windows(SASL.len())
            .any(|window| window == SASL.as_bytes())
    );
    WIPED.store(false, Ordering::SeqCst);
    WATCHED_LEN.store(line.len(), Ordering::SeqCst);
    WATCHED.store(line.as_ptr() as usize, Ordering::SeqCst);
    drop(line);
    drop(session);
    TARGET.store(0, Ordering::SeqCst);
    assert!(WIPED.load(Ordering::SeqCst));
    assert!(!LEAKED.load(Ordering::SeqCst));
}

#[cfg(feature = "unstable")]
#[test]
fn wipes_webpush_auth_and_combined_key_buffers() {
    const AUTH: &str = "d2VicHVzaC1hdXRoLXdpcGUtbWFya2VyLTgzNTc=";

    inspect(AUTH);
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .enable(papo_core::Feature::WebPush)
            .wanted_caps(["draft/webpush", "labeled-response"])
            .allow_insecure_auth(true)
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    while session.poll_transmit().is_some() {}
    session.handle_line(now, b"CAP * LS :draft/webpush labeled-response");
    while session.poll_transmit().is_some() {}
    session.handle_line(now, b"CAP * ACK :draft/webpush labeled-response");
    while session.poll_transmit().is_some() {}

    session
        .webpush_register(
            now,
            "https://push.example/subscription",
            "AQIDBAUGBwgJCgsMDQ4P_w",
            &Secret::new(AUTH),
        )
        .unwrap();
    let line = session.poll_transmit().unwrap();
    assert!(
        line.windows(AUTH.len())
            .any(|window| window == AUTH.as_bytes())
    );
    drop(line);
    drop(session);
    TARGET.store(0, Ordering::SeqCst);
    assert!(!LEAKED.load(Ordering::SeqCst));
}

#[cfg(feature = "unstable")]
#[test]
fn wipes_bouncer_password_attribute_buffers() {
    const PASSWORD: &str = "bouncer-password-wipe-marker-4921";

    inspect(PASSWORD);
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .enable(papo_core::Feature::Bouncer)
            .wanted_caps(["soju.im/bouncer-networks", "labeled-response"])
            .allow_insecure_auth(true)
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    while session.poll_transmit().is_some() {}
    session.handle_line(now, b"CAP * LS :soju.im/bouncer-networks labeled-response");
    while session.poll_transmit().is_some() {}
    session.handle_line(now, b"CAP * ACK :soju.im/bouncer-networks labeled-response");
    while session.poll_transmit().is_some() {}

    let password = papo_core::Secret::new(PASSWORD);
    let config = papo_core::__bouncer_attributes(
        Some("irc.example.org"),
        None,
        Some(6667),
        Some(false),
        None,
        None,
        None,
        Some(&password),
    )
    .unwrap();
    session.bouncer_add(now, config).unwrap();
    let line = session.poll_transmit().unwrap();
    assert!(
        line.windows(PASSWORD.len())
            .any(|window| window == PASSWORD.as_bytes())
    );
    drop(line);
    drop(session);
    drop(password);
    TARGET.store(0, Ordering::SeqCst);
    assert!(!LEAKED.load(Ordering::SeqCst));
}
