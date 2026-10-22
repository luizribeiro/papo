//! Account-creation example output against the runner-managed Ergo server.
#![cfg(not(target_family = "wasm"))]

use std::{process::Command, time::Duration};

use papo_test_server::{from_env, unique};
use tokio::time::timeout;

#[tokio::test(flavor = "current_thread")]
async fn creates_and_logs_in_during_connection_setup() {
    timeout(Duration::from_secs(10), async {
        let server = from_env();
        let account = unique("papo-create-account");
        let output = Command::new(env!("CARGO_BIN_EXE_create-account"))
            .arg(format!("irc://{}", server.plaintext_addr()))
            .env("IRC_NICK", &account)
            .env("IRC_PASSWORD", "q7Hx-2mZp_9vLcT4wRfB")
            .env("IRC_ALLOW_INSECURE_AUTH", "1")
            .output()
            .unwrap();

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
    .expect("live account-creation example did not finish within 10 seconds");
}
