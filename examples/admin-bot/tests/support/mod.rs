#![allow(dead_code)]

use std::{path::PathBuf, time::Duration};

use papo_test_server::ServerInfo;
use tokio::time::{sleep, timeout};

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) struct AdminBot {
    child: std::process::Child,
    password: PathBuf,
}

impl AdminBot {
    pub(super) fn start(server: &ServerInfo, nick: &str, channels: &str, admins: &str) -> Self {
        let password = std::env::temp_dir().join(format!("{nick}.password"));
        std::fs::write(&password, format!("{PASSWORD}\n")).unwrap();
        let address = format!("ircs://localhost:{}", server.tls_addr().port());
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_admin-bot"))
            .env("IRC_SERVER", address)
            .env("IRC_NICK", nick)
            .env("IRC_PASSWORD_FILE", &password)
            .env("IRC_CA_FILE", server.ca_pem_path())
            .env("IRC_CHANNELS", channels)
            .env("IRC_ADMINS", admins)
            .spawn()
            .unwrap();
        Self { child, password }
    }

    pub(super) async fn shutdown(&mut self) {
        let status = std::process::Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let status = timeout(Duration::from_secs(10), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    return status;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("admin bot did not exit within 10 seconds");
        assert_eq!(status.code(), Some(0));
    }
}

impl Drop for AdminBot {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.password);
    }
}
