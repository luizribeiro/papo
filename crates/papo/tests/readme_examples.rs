//! Checks that crate documentation links only to tracked examples.

#![cfg(not(all(target_os = "wasi", not(target_env = "p1"))))]

use std::{collections::BTreeSet, path::Path, process::Command};

const LIB: &str = include_str!("../src/lib.rs");

#[test]
fn crate_doc_examples_are_tracked() {
    let linked = LIB
        .split_once("//! # Examples\n")
        .expect("crate examples heading")
        .1
        .lines()
        .take_while(|line| !line.starts_with("//! # Cargo features"))
        .filter_map(|line| line.split_once("/examples/").map(|(_, path)| path))
        .map(|path| path.strip_suffix(')').expect("example link closes"))
        .collect::<BTreeSet<_>>();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new("git")
        .args(["ls-files", "--", "examples"])
        .current_dir(root)
        .output()
        .expect("git ls-files runs");
    assert!(output.status.success());
    let files = String::from_utf8(output.stdout).expect("git paths are UTF-8");
    let tracked = files
        .lines()
        .filter_map(|path| path.strip_prefix("examples/"))
        .filter_map(|path| path.split_once('/').map(|(directory, _)| directory))
        .collect::<BTreeSet<_>>();

    assert_eq!(
        linked, tracked,
        "crate example links must match tracked examples"
    );
}
