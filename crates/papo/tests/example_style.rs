//! Source-level guards for example readability.
#![cfg(not(target_family = "wasm"))]

use std::path::Path;

const README: &str = include_str!("../../../README.md");
const INTERNAL_CRATE_NAMES: &[&str] = &[
    "papo_proto",
    "papo_core",
    "papo_bot",
    "papo-proto",
    "papo-core",
    "papo-bot",
];

#[test]
fn examples_do_not_construct_names_with_default_casemapping() {
    check_directory(Path::new("../../examples"));
    check_directory(Path::new("examples"));
}

#[test]
fn examples_only_name_the_papo_facade() {
    check_public_text(Path::new("../../examples"));
    check_public_text(Path::new("examples"));
}

#[test]
fn readme_only_imports_the_papo_facade() {
    for name in &INTERNAL_CRATE_NAMES[..3] {
        assert!(!README.contains(name), "README names internal crate {name}");
    }
}

fn check_directory(directory: &Path) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            check_directory(&path);
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            let forbidden = concat!("Casemapping", "::default()");
            assert!(
                !source.contains(forbidden),
                "{} calls {forbidden}",
                path.display()
            );
        }
    }
}

fn check_public_text(directory: &Path) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            check_public_text(&path);
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for name in INTERNAL_CRATE_NAMES {
            assert!(
                !source.contains(name),
                "{} names internal crate {name}",
                path.display()
            );
        }
    }
}
