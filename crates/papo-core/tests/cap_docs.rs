//! Keeps generated capability documentation synchronized with the registry.

#![cfg(all(not(target_os = "wasi"), feature = "unstable"))]

use std::path::{Path, PathBuf};

const START: &str = "<!-- caps:start -->";
const END: &str = "<!-- caps:end -->";

fn section(source: &str, rustdoc: bool) -> String {
    let start = source.find(START).unwrap() + START.len();
    let end = source[start..].find(END).unwrap() + start;
    source[start..end]
        .lines()
        .map(|line| {
            if rustdoc {
                line.strip_prefix("//! ")
                    .or_else(|| line.strip_prefix("//!"))
                    .unwrap_or(line)
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn replace(path: &Path, rustdoc: bool, table: &str) {
    let source = std::fs::read_to_string(path).unwrap();
    let start = source.find(START).unwrap() + START.len();
    let end = source[start..].find(END).unwrap() + start;
    let table = if rustdoc {
        table
            .lines()
            .map(|line| format!("//! {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        table.trim_end().to_owned()
    };
    let suffix = if rustdoc {
        source[..end].rfind('\n').map_or(0, |index| index + 1)
    } else {
        end
    };
    let updated = format!("{}\n{}\n{}", &source[..start], table, &source[suffix..]);
    std::fs::write(path, updated).unwrap();
}

#[test]
fn crate_docs_and_readme_match_the_capability_registry() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = crate_dir.parent().unwrap().parent().unwrap();
    let readme = root.join("README.md");
    let lib = crate_dir.join("src/lib.rs");
    let expected = papo_core::cap::markdown_table();
    if std::env::var_os("PAPO_BLESS").is_some() {
        replace(&readme, false, &expected);
        replace(&lib, true, &expected);
    }
    assert_eq!(
        section(&std::fs::read_to_string(readme).unwrap(), false),
        expected.trim()
    );
    assert_eq!(
        section(&std::fs::read_to_string(lib).unwrap(), true),
        expected.trim()
    );
}
