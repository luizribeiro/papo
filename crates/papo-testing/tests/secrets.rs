//! Profile-independent regressions for secret redaction and zeroization.

#![allow(unsafe_code)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use papo::{Line, Secret};
use papo_core::{Config, RateLimit, Session};

struct InspectingAllocator;

static WATCHED: AtomicUsize = AtomicUsize::new(0);
static WATCHED_LEN: AtomicUsize = AtomicUsize::new(0);
static WIPED: AtomicBool = AtomicBool::new(false);

// SAFETY: Allocation operations are forwarded unchanged to the system allocator. Deallocation
// inspects the still-live allocation before forwarding the original pointer and layout.
unsafe impl GlobalAlloc for InspectingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The original allocation layout is forwarded unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer as usize == WATCHED.load(Ordering::SeqCst) {
            let length = WATCHED_LEN.load(Ordering::SeqCst);
            // SAFETY: The allocation remains live until the forwarded deallocation below.
            let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
            WIPED.store(bytes.iter().all(|byte| *byte == 0), Ordering::SeqCst);
            WATCHED.store(0, Ordering::SeqCst);
        }
        // SAFETY: The original pointer and layout are forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: InspectingAllocator = InspectingAllocator;

fn watch(pointer: *const u8, length: usize) {
    WIPED.store(false, Ordering::SeqCst);
    WATCHED_LEN.store(length, Ordering::SeqCst);
    WATCHED.store(pointer as usize, Ordering::SeqCst);
}

#[test]
fn queued_secrets_are_redacted_without_hiding_ordinary_lines() {
    const PASSWORD: &str = "checklist-pass-marker-7391";
    let now = Instant::now();
    let mut session = Session::new(
        Config::default()
            .password(papo::Secret::new(PASSWORD))
            .allow_insecure_auth(true)
            .rate_limit(RateLimit::unlimited()),
    );
    session.handle_connected(now);
    session.handle_line(now, b"CAP * LS :message-tags");
    session
        .send(
            now,
            Line::new("PRIVMSG")
                .param("#papo")
                .param("ordinary debug marker"),
        )
        .unwrap();

    let debug = format!("{session:?}");
    let decimal = PASSWORD
        .bytes()
        .map(|byte| byte.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        debug.contains("PASS <redacted>"),
        "missing redacted PASS: {debug}"
    );
    assert!(
        debug.contains("PRIVMSG #papo :ordinary debug marker"),
        "ordinary line hidden: {debug}"
    );
    assert!(!debug.contains(PASSWORD), "plaintext secret in {debug}");
    assert!(!debug.contains(&decimal), "decimal secret bytes in {debug}");
    assert!(
        !debug.contains(&STANDARD.encode(PASSWORD)),
        "base64 secret in {debug}"
    );
}

#[test]
fn secret_and_wire_buffers_are_wiped_on_drop() {
    let secret = Secret::new("checklist-secret-wipe-marker-6184".to_owned());
    watch(secret.expose().as_ptr(), secret.expose().len());
    drop(secret);
    assert!(WIPED.load(Ordering::SeqCst));

    let line = Line::new("PASS")
        .param(Secret::new("checklist-wire-wipe-marker-4729"))
        .encode()
        .unwrap();
    watch(line.as_ptr(), line.len());
    drop(line);
    assert!(WIPED.load(Ordering::SeqCst));
}
