use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use tokio::time::sleep;

pub(super) struct BouncerExample {
    child: std::process::Child,
    stdout: PathBuf,
}

impl BouncerExample {
    pub(super) fn start(
        url: &str,
        upstream: &str,
        nick: &str,
        account: &str,
        password: &str,
    ) -> Self {
        let directory = std::env::temp_dir();
        let stdout = directory.join(format!("{nick}.stdout"));
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_bouncer-networks"))
            .args([url, upstream])
            .env("IRC_NICK", nick)
            .env("IRC_ACCOUNT", account)
            .env("IRC_PASSWORD", password)
            .stdout(Stdio::from(std::fs::File::create(&stdout).unwrap()))
            .spawn()
            .unwrap();
        Self { child, stdout }
    }

    pub(super) async fn wait_for(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let output = std::fs::read_to_string(&self.stdout).unwrap();
            if output.contains(text) {
                return;
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("example exited with {status}");
            }
            assert!(
                Instant::now() < deadline,
                "example did not print {text:?}; stdout: {output:?}"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub(super) async fn finish(&mut self) -> String {
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            sleep(Duration::from_millis(10)).await;
        };
        assert!(status.success(), "example exited with {status}");
        std::fs::read_to_string(&self.stdout).unwrap()
    }
}

impl Drop for BouncerExample {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.stdout);
    }
}
