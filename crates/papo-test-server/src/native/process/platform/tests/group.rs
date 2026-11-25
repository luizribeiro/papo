use std::{
    env, fs, io,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::native::{directory::TemporaryDirectory, process::ServerChild};

#[test]
fn kills_the_server_tree_on_orderly_stop() {
    let directory = TemporaryDirectory::create().unwrap();
    let mut owner = Command::new(env::current_exe().unwrap())
        .args(["--ignored", "::fixture_owner"])
        .env("PAPO_WATCHDOG_TEST_DIR", directory.path())
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();

    let ready = directory.path().join("pids");
    let entries = within(|| {
        fs::read_to_string(&ready)
            .ok()
            .filter(|text| text.ends_with('\n'))
    });
    let pids: Vec<_> = entries.split_whitespace().collect();
    assert_eq!(pids.len(), 2);
    assert!(pids.iter().copied().all(exists));

    drop(owner.stdin.take());
    let status = within(|| owner.try_wait().unwrap());
    assert!(status.success(), "{status}");
    within(|| pids.iter().all(|pid| !exists(pid)).then_some(()));
}

fn exists(pid: &str) -> bool {
    Command::new("sh")
        .args(["-c", r#"kill -0 "$0" 2>/dev/null"#, pid])
        .status()
        .unwrap()
        .success()
}

fn within<T>(mut observe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = observe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for fixture");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "run in a dedicated fixture owner process"]
fn fixture_owner() {
    let Some(directory) = env::var_os("PAPO_WATCHDOG_TEST_DIR") else {
        return;
    };
    let mut server = ServerChild::spawn(Command::new("sh").current_dir(directory).args([
        "-c",
        r#"trap '' TERM
           sleep 60 &
           worker=$!
           printf '%s %s\n' "$$" "$worker" > pids
           wait "$worker""#,
    ]))
    .unwrap();
    io::stdin().read_line(&mut String::new()).unwrap();
    server.stop().unwrap();
}
