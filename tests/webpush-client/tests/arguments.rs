//! Argument handling for the Web Push fixture.
#![cfg(not(target_family = "wasm"))]

use std::process::Command;

#[test]
fn register_requires_an_account() {
    let output = Command::new(env!("CARGO_BIN_EXE_papo-webpush-test-client"))
        .args(["register", "irc://127.0.0.1"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "Error: \"missing account\""
    );
}
