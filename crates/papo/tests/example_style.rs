//! Source-level guards for example readability.
#![cfg(not(target_family = "wasm"))]

use std::path::Path;

#[test]
fn examples_do_not_construct_names_with_default_casemapping() {
    check_directory(Path::new("../../examples"));
    check_directory(Path::new("examples"));
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
