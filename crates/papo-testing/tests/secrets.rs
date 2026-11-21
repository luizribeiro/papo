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
        .send(now, "PRIVMSG #papo :ordinary debug marker".parse().unwrap())
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

#[tokio::test]
async fn the_echo_of_a_secret_request_stays_redacted() {
    const PASSWORD: &str = "services-password-5129";
    let (client, mut events, mut wire) =
        echo_wire(&["echo-message", "labeled-response", "message-tags"]).await;
    let request = tokio::spawn(async move {
        client
            .request_raw(
                Line::new("PRIVMSG")
                    .param("NickServ")
                    .param(Secret::new(format!("IDENTIFY alice {PASSWORD}"))),
            )
            .await
    });
    assert_eq!(
        wire.next_sent().await,
        format!("@label=0 PRIVMSG NickServ :IDENTIFY alice {PASSWORD}")
    );
    wire.server(format!(
        "@label=0;msgid=RfcBTzyxY120006;time=2026-11-16T09:12:01.123Z :papo-wire!~papo@client.example PRIVMSG NickServ :IDENTIFY alice {PASSWORD}"
    ))
    .await;
    wire.server(":NickServ!NickServ@localhost NOTICE papo-wire :You're now logged in as alice")
        .await;

    let lines = request.await.unwrap().unwrap();
    assert!(
        format!("{lines:?}").ends_with("PRIVMSG NickServ :<redacted>]"),
        "{lines:?}"
    );
    let mut seen = Vec::new();
    while !format!("{seen:?}").contains("now logged in") {
        seen.push(events.next().await.expect("driver stopped").0);
    }
    assert!(!format!("{seen:?}").contains(PASSWORD), "{seen:?}");
}

async fn echo_wire(caps: &[&str]) -> (papo::Client, papo::Events, papo_testing::Wire) {
    let (client, events, mut wire) = papo_testing::wire(
        papo::Client::builder("irc://offline.test", "papo-wire")
            .allow_insecure_auth(true)
            .rate_limit(papo::RateLimit::unlimited())
            .reconnect(papo::Reconnect::never()),
    );
    wire.register(caps).await;
    (client, events, wire)
}

async fn events_until(events: &mut papo::Events, marker: &str) -> String {
    let mut seen = Vec::new();
    while !format!("{seen:?}").contains(marker) {
        seen.push(events.next().await.expect("driver stopped").0);
    }
    format!("{seen:?}")
}

#[tokio::test]
async fn the_unlabelled_echo_of_a_secret_send_stays_redacted() {
    const PASSWORD: &str = "services-password-5129";
    let (client, mut events, mut wire) =
        echo_wire(&["echo-message", "labeled-response", "message-tags"]).await;
    client
        .send_raw(
            Line::new("PRIVMSG")
                .param("NickServ")
                .param(Secret::new(format!("IDENTIFY alice {PASSWORD}"))),
        )
        .await
        .unwrap();
    assert_eq!(
        wire.next_sent().await,
        format!("PRIVMSG NickServ :IDENTIFY alice {PASSWORD}")
    );
    wire.server(format!(
        "@msgid=RfcBTzyxY120006;time=2026-11-16T09:12:01.123Z :papo-wire!~papo@client.example PRIVMSG NickServ :IDENTIFY alice {PASSWORD}"
    ))
    .await;
    wire.server(":NickServ!NickServ@localhost NOTICE papo-wire :You're now logged in as alice")
        .await;

    let seen = events_until(&mut events, "now logged in").await;
    assert!(!seen.contains(PASSWORD), "{seen}");
}

#[tokio::test]
async fn the_echo_of_a_secret_redaction_reason_stays_redacted() {
    for caps in [
        [
            "draft/message-redaction",
            "echo-message",
            "labeled-response",
            "message-tags",
        ]
        .as_slice(),
        [
            "draft/message-redaction",
            "labeled-response",
            "message-tags",
        ]
        .as_slice(),
    ] {
        assert_secret_redaction_echo_stays_redacted(caps).await;
    }
}

async fn assert_secret_redaction_echo_stays_redacted(caps: &[&str]) {
    const REASON: &str = "pasted the staff key services-password-5129";
    let (client, mut events, mut wire) = echo_wire(caps).await;
    let request = tokio::spawn(async move {
        client
            .request_raw(
                Line::new("REDACT")
                    .param("#papo")
                    .param("b56bhkz4gmj7ig2jeg9hs7d2cs")
                    .param(Secret::new(REASON)),
            )
            .await
    });
    assert_eq!(
        wire.next_sent().await,
        format!("@label=0 REDACT #papo b56bhkz4gmj7ig2jeg9hs7d2cs :{REASON}")
    );
    wire.server(format!(
        "@msgid=qsgqkhqcmnxm6x2dcricaf69f6;time=2026-11-16T07:39:12.791Z :papo-wire!~papo@client.example REDACT #papo b56bhkz4gmj7ig2jeg9hs7d2cs :{REASON}"
    ))
    .await;
    wire.server("@time=2026-11-16T07:39:12.791Z;label=0 :ergo.test ACK")
        .await;
    wire.server(":ergo.test NOTICE papo-wire :redaction marker")
        .await;

    let lines = request.await.unwrap().unwrap();
    assert!(!format!("{lines:?}").contains(REASON), "{lines:?}");
    let seen = events_until(&mut events, "redaction marker").await;
    assert!(seen.contains("b56bhkz4gmj7ig2jeg9hs7d2cs"), "{seen}");
    assert!(!seen.contains("services-password-5129"), "{seen}");
}
