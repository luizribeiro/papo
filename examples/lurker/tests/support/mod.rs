#![allow(dead_code)]

use std::{path::PathBuf, process::Stdio, time::Duration};

use tokio::time::sleep;

pub(super) struct Lurker {
    child: std::process::Child,
    output: PathBuf,
}

impl Lurker {
    pub(super) fn start(server: &str, nick: &str) -> Self {
        let output = std::env::temp_dir().join(format!("{nick}.stdout"));
        let stdout = std::fs::File::create(&output).unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_lurker"))
            .arg(server)
            .env("IRC_NICK", nick)
            .stdout(Stdio::from(stdout))
            .spawn()
            .unwrap();
        Self { child, output }
    }

    pub(super) async fn wait_for(&self, text: &str) {
        while !std::fs::read_to_string(&self.output)
            .unwrap()
            .contains(text)
        {
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
        let _ = std::fs::remove_file(&self.output);
    }
}
