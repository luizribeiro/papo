#![allow(dead_code)]

use std::{path::PathBuf, process::Stdio, time::Duration};

use papo_test_server::ServerInfo;
use tokio::time::sleep;

pub(super) const PASSWORD: &str = "q7Hx-2mZp_9vLcT4wRfB";

pub(super) struct Lurker {
    child: std::process::Child,
    password: PathBuf,
    output: PathBuf,
}

impl Lurker {
    pub(super) fn start(server: &ServerInfo, nick: &str, channel: &str) -> Self {
        let password = std::env::temp_dir().join(format!("{nick}.password"));
        let output = std::env::temp_dir().join(format!("{nick}.stdout"));
        std::fs::write(&password, format!("{PASSWORD}\n")).unwrap();
        let stdout = std::fs::File::create(&output).unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_lurker"))
            .env("IRC_SERVER", "localhost")
            .env("IRC_PORT", server.tls_addr().port().to_string())
            .env("IRC_NICK", nick)
            .env("IRC_PASSWORD_FILE", &password)
            .env("IRC_CA_FILE", server.ca_pem_path())
            .env("IRC_CHANNELS", channel)
            .stdout(Stdio::from(stdout))
            .spawn()
            .unwrap();
        Self {
            child,
            password,
            output,
        }
    }

    pub(super) async fn wait_for(&self, text: &str) {
        loop {
            if std::fs::read_to_string(&self.output)
                .unwrap()
                .contains(text)
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub(super) fn finish(&mut self) -> String {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        std::fs::read_to_string(&self.output).unwrap()
    }
}

impl Drop for Lurker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.password);
        let _ = std::fs::remove_file(&self.output);
    }
}
