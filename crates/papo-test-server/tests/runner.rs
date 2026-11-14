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
    for server in ["inspircd", "solanum", "unrealircd"] {
        let directory = directory();
        let helper = executable(
            &directory,
            "server-helper",
            &format!(
                r#"test "$PAPO_TEST_SERVER" = {server}
printf '%s\n' 127.0.0.1:6667 127.0.0.1:6697 ws://127.0.0.1:8097/ wss://localhost:8098/ fixture-ca.pem '' '' '' ''
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
fn live_binary_forwards_profiles_and_maps_the_legacy_ergo_name() {
    for (server, selected, expected) in [
        ("ergo", None, "ergo-default"),
        ("ergo", Some("ergo-persistent"), "ergo-persistent"),
        ("inspircd", Some("inspircd"), "inspircd"),
    ] {
        let directory = directory();
        let helper = executable(
            &directory,
            "server-helper",
            &format!(
                r#"test "$PAPO_TEST_PROFILE" = {expected}
printf '%s\n' 127.0.0.1:6667 127.0.0.1:6697 ws://127.0.0.1:8097/ wss://localhost:8098/ fixture-ca.pem '' '' '' ''
read -r _ || true"#
            ),
        );
        let binary = executable(
            &directory,
            "live_profile-deadbeef",
            &format!("test \"$PAPO_TEST_PROFILE\" = {expected}"),
        );
        let mut command = runner(&binary);
        command
            .env("PAPO_TEST_SERVER", server)
            .env("PAPO_TEST_SERVER_BIN", helper);
        if let Some(selected) = selected {
            command.env("PAPO_TEST_PROFILE", selected);
        }
        let status = command.status().unwrap();
        assert!(status.success());
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn live_binary_receives_complete_bouncer_metadata() {
    let directory = directory();
    let helper = executable(
        &directory,
        "server-helper",
        r"printf '%s\n' 127.0.0.1:6667 127.0.0.1:6697 ws://127.0.0.1:8097/ wss://localhost:8098/ fixture-ca.pem papo-bouncer bouncer-password /tmp/soju.conf 127.0.0.1:7667
read -r _ || true",
    );
    let binary = executable(
        &directory,
        "live_bouncer-deadbeef",
        r#"test "$PAPO_TEST_BOUNCER_USER" = papo-bouncer
test "$PAPO_TEST_BOUNCER_PASSWORD" = bouncer-password
test "$PAPO_TEST_BOUNCER_CONFIG" = /tmp/soju.conf
test "$PAPO_TEST_UPSTREAM" = 127.0.0.1:7667"#,
    );
    let status = runner(&binary)
        .env("PAPO_TEST_PROFILE", "soju-ergo")
        .env("PAPO_TEST_SERVER_BIN", helper)
        .status()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn starts_a_server_only_for_invocations_that_run_live_tests() {
    for (binary, args, starts) in [
        ("papo-deadbeef", "", true),
        ("papo-deadbeef", "--nocapture", true),
        ("papo-deadbeef", "--list --format terse", false),
        ("papo-deadbeef", "--list --format terse --ignored", false),
        (
            "papo-deadbeef",
            "--exact driver::tests::mode_changes_return_confirmed_facts_after_a_later_disconnect --nocapture",
            false,
        ),
        (
            "papo-deadbeef",
            "--exact handle::tests::only_a_live_own_line_recaptures_a_failed_event_proof --nocapture",
            false,
        ),
        (
            "papo-deadbeef",
            "--exact transport::live::plaintext_cap_exchange --nocapture",
            true,
        ),
        (
            "papo-deadbeef",
            "--exact transport::websocket::live::rejects_a_partial_line_at_shutdown --nocapture",
            true,
        ),
        (
            "papo-deadbeef",
            "builder::tests::fills_every_session_configuration_field --exact",
            false,
        ),
        (
            "papo-deadbeef",
            "--exact transport::live_helpers::connect --nocapture",
            false,
        ),
        (
            "papo-deadbeef",
            "--exact builder::tests::a transport::live::plaintext_cap_exchange",
            true,
        ),
        (
            "papo-deadbeef",
            "--exact builder::tests::a --skip transport::live::plaintext_cap_exchange",
            false,
        ),
        (
            "papo-deadbeef",
            "--exact --format terse --test-threads 1 --color always --logfile out --shuffle-seed 7 -Z unstable-options",
            true,
        ),
        ("papo-deadbeef", "--exact", true),
        ("papo-deadbeef", "--exact builder::tests::a --format", false),
        (
            "papo-deadbeef",
            "--list transport::live::plaintext_cap_exchange",
            false,
        ),
        ("papo-deadbeef", "plaintext_cap_exchange", true),
        ("papo-deadbeef", "builder::tests", true),
        ("live_client-deadbeef", "", true),
        (
            "live_client-deadbeef",
            "--exact registers_messages_itself_and_quits --nocapture",
            true,
        ),
        ("live_client-deadbeef", "--list --format terse", false),
        ("parser_tests-deadbeef", "", false),
        (
            "parser_tests-deadbeef",
            "--exact transport::live::plaintext_cap_exchange",
            false,
        ),
    ] {
        let directory = directory();
        let started = directory.join("started");
        let helper = executable(
            &directory,
            "server-helper",
            &format!(
                r"touch {}
printf '%s\n' 127.0.0.1:6667 127.0.0.1:6697 ws://127.0.0.1:8097/ wss://localhost:8098/ fixture-ca.pem '' '' '' ''
read -r _ || true",
                started.display()
            ),
        );
        let check = if starts {
            "test \"$PAPO_TEST_PLAINTEXT\" = 127.0.0.1:6667"
        } else {
            "test -z \"${PAPO_TEST_PLAINTEXT-}\""
        };
        let binary = executable(&directory, binary, check);
        let status = runner(&binary)
            .args(args.split_whitespace())
            .env("PAPO_TEST_SERVER_BIN", helper)
            .status()
            .unwrap();
        assert!(status.success(), "{binary:?} {args:?}");
        assert_eq!(started.exists(), starts, "{binary:?} {args:?}");
        fs::remove_dir_all(directory).unwrap();
    }
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
        .env_remove("PAPO_TEST_BOUNCER_USER")
        .env_remove("PAPO_TEST_BOUNCER_PASSWORD")
        .env_remove("PAPO_TEST_BOUNCER_CONFIG")
        .env_remove("PAPO_TEST_UPSTREAM")
        .env_remove("PAPO_TEST_SERVER")
        .env_remove("PAPO_TEST_PROFILE");
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
