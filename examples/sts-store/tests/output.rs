//! Output test for the STS store example.
#![cfg(not(all(target_os = "wasi", not(target_env = "p1"))))]

use std::{fs, process::Command};

#[test]
fn persists_a_policy_across_reopening() {
    let path = std::env::temp_dir().join(format!("papo-sts-store-{}.txt", std::process::id()));
    let _ = fs::remove_file(&path);
    let output = Command::new(env!("CARGO_BIN_EXE_papo-sts-store"))
        .arg(&path)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "irc.example.net requires TLS on port 6697\nirc.libera.chat policy removed\n"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "irc.example.net\t6697\t4000000000\t15552000\n"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn rejects_a_partial_policy_file() {
    let path = std::env::temp_dir().join(format!("papo-sts-store-bad-{}.txt", std::process::id()));
    fs::write(&path, "irc.example.net\t6697\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_papo-sts-store"))
        .arg(&path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "irc.example.net\t6697\n"
    );
    fs::remove_file(path).unwrap();
}
