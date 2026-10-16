//! Register-example output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{process::Command, time::Duration};

use papo_test_server::{from_env, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn registers_and_logs_in_during_connection_setup() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let account = unique("papo-register");
        let password = std::env::temp_dir().join(format!("{account}.password"));
        std::fs::write(&password, "q7Hx-2mZp_9vLcT4wRfB\n").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_register"))
            .env("IRC_SERVER", server.plaintext_addr().ip().to_string())
            .env("IRC_PORT", server.plaintext_addr().port().to_string())
            .env("IRC_PLAINTEXT", "1")
            .env("IRC_ACCOUNT", &account)
            .env("IRC_PASSWORD_FILE", &password)
            .output()
            .unwrap();
        std::fs::remove_file(password).unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("logged in as {account}\n")
        );
    })
    .await
    .expect("live registration example did not finish within 10 seconds");
}
