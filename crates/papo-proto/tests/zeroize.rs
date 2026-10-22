//! Allocator-level regression coverage for wiping secret allocations before release.

#![allow(unsafe_code)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    ptr,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use papo_proto::Secret;

struct InspectingAllocator;

static WATCHED: AtomicUsize = AtomicUsize::new(0);
static WATCHED_LEN: AtomicUsize = AtomicUsize::new(0);
static WIPED: AtomicBool = AtomicBool::new(false);

// SAFETY: Every allocation operation is forwarded to `System` with its original pointer and
// layout; inspection happens before forwarding deallocation.
unsafe impl GlobalAlloc for InspectingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The system allocator receives the unchanged allocation layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer as usize == WATCHED.load(Ordering::SeqCst) {
            let length = WATCHED_LEN.load(Ordering::SeqCst);
            // SAFETY: The watched pointer and length came from this still-live allocation, and
            // GlobalAlloc calls dealloc before releasing it to the system allocator.
            let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
            WIPED.store(bytes.iter().all(|byte| *byte == 0), Ordering::SeqCst);
            WATCHED.store(0, Ordering::SeqCst);
        }
        // SAFETY: The pointer and layout are forwarded unchanged to their originating allocator.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: InspectingAllocator = InspectingAllocator;

fn watch(secret: &Secret) {
    WIPED.store(false, Ordering::SeqCst);
    WATCHED_LEN.store(secret.expose_bytes().len(), Ordering::SeqCst);
    WATCHED.store(secret.expose_bytes().as_ptr() as usize, Ordering::SeqCst);
}

fn assert_wiped(secret: Secret) {
    watch(&secret);
    drop(secret);
    assert!(WIPED.load(Ordering::SeqCst));
}

#[test]
fn wipes_owned_secret_buffers_on_their_last_drop() {
    assert_wiped(Secret::new("text-secret-marker-8193"));
    assert_wiped(Secret::from_bytes(b"byte-secret-marker-2754".to_vec()));
    assert_wiped(Secret::from(String::from("string-secret-marker-6248")));
    assert_wiped(Secret::from(Box::<str>::from("box-secret-marker-4637")));
    assert_wiped(Secret::from(std::borrow::Cow::Owned(String::from(
        "cow-secret-marker-9516",
    ))));

    let first = Secret::new("shared-secret-marker-3072");
    let last = first.clone();
    watch(&first);
    drop(first);
    assert!(!WIPED.load(Ordering::SeqCst));
    assert_eq!(last.expose(), Some("shared-secret-marker-3072"));
    drop(last);
    assert!(WIPED.load(Ordering::SeqCst));

    assert_eq!(WATCHED.load(Ordering::SeqCst), ptr::null::<u8>() as usize);
}
