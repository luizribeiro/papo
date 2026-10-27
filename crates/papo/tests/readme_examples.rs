//! Checks that crate documentation links every example crate.

#![cfg(not(all(target_os = "wasi", not(target_env = "p1"))))]

use std::{collections::BTreeSet, path::Path};

const LIB: &str = include_str!("../src/lib.rs");

#[test]
fn crate_doc_examples_are_present() {
    let linked = LIB
        .split_once("//! # Examples\n")
        .expect("crate examples heading")
        .1
        .lines()
        .take_while(|line| !line.starts_with("//! # Cargo features"))
        .filter_map(|line| line.split_once("/examples/").map(|(_, path)| path))
        .map(|path| path.strip_suffix(')').expect("example link closes"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let present = std::fs::read_dir(root.join("examples"))
        .expect("examples directory is readable")
        .map(|entry| entry.expect("example directory entry is readable"))
        .filter(|entry| entry.path().join("Cargo.toml").is_file())
        .map(|entry| {
            entry
                .file_name()
                .into_string()
                .expect("example directory names are UTF-8")
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(
        linked, present,
        "crate example links must match present example crates"
    );
}
