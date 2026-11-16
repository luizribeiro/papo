//! Output test for the STS store example.
#![cfg(not(target_family = "wasm"))]

use std::{fs, path::PathBuf, process::Output};

const SAVED_SAMPLE: &str = "irc.example.net\t6697\t4000000000\t15552000\ttcp";

struct PolicyFile(PathBuf);

impl PolicyFile {
    fn new(name: &str, contents: Option<&str>) -> Self {
        let path = std::env::temp_dir().join(format!("papo-sts-{name}-{}.txt", std::process::id()));
        match contents {
            Some(contents) => fs::write(&path, contents).unwrap(),
            None => drop(fs::remove_file(&path)),
        }
        Self(path)
    }

    fn run_example(&self) -> Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_papo-sts-store"))
            .arg("ircs://irc.example.net")
            .arg(&self.0)
            .output()
            .unwrap()
    }

    fn contents(&self) -> String {
        fs::read_to_string(&self.0).unwrap()
    }

    fn sorted_lines(&self) -> Vec<String> {
        let mut lines = self
            .contents()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        lines.sort();
        lines
    }
}

impl Drop for PolicyFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn persists_a_policy_across_reopening() {
    let file = PolicyFile::new("fresh", None);
    let output = file.run_example();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "irc.example.net requires TLS on port 6697\n"
    );
    assert_eq!(file.sorted_lines(), [SAVED_SAMPLE]);
}

#[test]
fn keeps_stored_tcp_and_websocket_policies_unchanged() {
    // Libera advertises `sts=duration=2592000,port=6697`; Ergo's WebSocket listener sits on 443.
    let libera = "irc.libera.chat\t6697\t1798761600\t2592000\ttcp";
    let ergo = "irc.ergo.chat\t443\t1830297600\t31536000\twebsocket";
    let file = PolicyFile::new("stored", Some(&format!("{libera}\n{ergo}\n")));
    let output = file.run_example();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(file.sorted_lines(), [ergo, SAVED_SAMPLE, libera]);
}

#[test]
fn rejects_an_unknown_transport() {
    let contents = "irc.libera.chat\t6697\t1798761600\t2592000\tsctp\n";
    let file = PolicyFile::new("transport", Some(contents));
    let output = file.run_example();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid STS policy transport"));
    assert_eq!(file.contents(), contents);
}

#[test]
fn rejects_a_partial_policy_file() {
    let contents = "irc.example.net\t6697\n";
    let file = PolicyFile::new("partial", Some(contents));
    let output = file.run_example();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid STS policy"));
    assert_eq!(file.contents(), contents);
}
