//! Contract tests for the native test-runner process lifecycle.
#![cfg(not(target_family = "wasm"))]

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn failing_live_binary_stops_the_server_and_preserves_101() {
    let directory = directory();
    let pid_file = directory.join("ergo.pid");
    let binary = executable(&directory, "live_failure-deadbeef", "exit 101");
    let status = runner(&binary)
        .env("PAPO_TEST_ERGO_PID_FILE", &pid_file)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(101));
    let pid = fs::read_to_string(pid_file).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn live_binary_respects_preset_server_variables() {
    let directory = directory();
    let binary = executable(
        &directory,
        "live_preset-deadbeef",
        "test \"$PAPO_TEST_PLAINTEXT\" = 192.0.2.1:6667 && test \"$PAPO_TEST_TLS\" = 192.0.2.1:6697 && test \"$PAPO_TEST_WS\" = ws://192.0.2.1:8097/ && test \"$PAPO_TEST_WSS\" = wss://irc.example:8098/webirc && test \"$PAPO_TEST_CA\" = fixture-ca.pem",
    );
    let status = runner(&binary)
        .env("PAPO_TEST_SERVER_BIN", directory.join("must-not-run"))
        .env("PAPO_TEST_PLAINTEXT", "192.0.2.1:6667")
        .env("PAPO_TEST_TLS", "192.0.2.1:6697")
        .env("PAPO_TEST_WS", "ws://192.0.2.1:8097/")
        .env("PAPO_TEST_WSS", "wss://irc.example:8098/webirc")
        .env("PAPO_TEST_CA", "fixture-ca.pem")
        .status()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn live_binary_forwards_the_selected_server_kind() {
    for server in ["inspircd", "unrealircd"] {
        let directory = directory();
        let helper = executable(
            &directory,
            "server-helper",
            &format!(
                r#"test "$PAPO_TEST_SERVER" = {server}
printf '%s\n' 127.0.0.1:6667 127.0.0.1:6697 ws://127.0.0.1:8097/ wss://localhost:8098/ fixture-ca.pem
read -r _ || true"#
            ),
        );
        let binary = executable(
            &directory,
            "live_selected-deadbeef",
            &format!("test \"$PAPO_TEST_SERVER\" = {server}"),
        );
        let status = runner(&binary)
            .env("PAPO_TEST_SERVER", server)
            .env("PAPO_TEST_SERVER_BIN", helper)
            .status()
            .unwrap();
        assert!(status.success());
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn plain_binary_does_not_start_a_server() {
    let directory = directory();
    let binary = executable(&directory, "parser_tests-deadbeef", "exit 0");
    let status = runner(&binary)
        .env("PAPO_TEST_SERVER_BIN", directory.join("must-not-run"))
        .status()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(directory).unwrap();
}

fn runner(binary: &Path) -> Command {
    let mut command = Command::new("bash");
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../nix/test-runner"))
        .arg(binary)
        .env_remove("PAPO_TEST_PLAINTEXT")
        .env_remove("PAPO_TEST_TLS")
        .env_remove("PAPO_TEST_WS")
        .env_remove("PAPO_TEST_WSS")
        .env_remove("PAPO_TEST_CA")
        .env_remove("PAPO_TEST_SERVER");
    command
}

fn executable(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    let output = Command::new("bash")
        .args(["-c", "command -v bash"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let bash = String::from_utf8(output.stdout).unwrap();
    fs::write(
        &path,
        format!("#!{}\nset -euo pipefail\n{body}\n", bash.trim()),
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn directory() -> PathBuf {
    let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!("papo-runner-test-{}-{id}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}
