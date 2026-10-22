//! Allocator-level regression coverage for queued and returned secret wire lines.

#![allow(unsafe_code)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

use papo_core::{Config, RateLimit, Secret, Session};
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

    inspect(QUEUED);
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session
        .send(Instant::now(), Line::new("PASS").param(Secret::new(QUEUED)))
        .unwrap();
    drop(session);
    assert!(!LEAKED.load(Ordering::SeqCst));

    inspect(POLLED);
    let mut session = Session::new(Config::default().rate_limit(RateLimit::unlimited()));
    session
        .send(Instant::now(), Line::new("PASS").param(Secret::new(POLLED)))
        .unwrap();
    let line = session.poll_transmit().unwrap();
    WATCHED_LEN.store(line.len(), Ordering::SeqCst);
    WATCHED.store(line.as_ptr() as usize, Ordering::SeqCst);
    drop(line);
    TARGET.store(0, Ordering::SeqCst);
    assert!(WIPED.load(Ordering::SeqCst));
    assert!(!LEAKED.load(Ordering::SeqCst));
}
