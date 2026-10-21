//! Checks that duplicated user-facing documentation stays synchronized.

const README: &str = include_str!("../../../README.md");
const LIB: &str = include_str!("../src/lib.rs");

#[test]
fn readme_quick_start_is_the_crate_doctest() {
    let readme = README
        .split_once("## Quick start\n\n```rust\n")
        .expect("README quick start")
        .1
        .split_once("\n```")
        .expect("README quick-start closing fence")
        .0;
    let crate_docs = LIB
        .split_once("//! # Quick start\n//!\n//! ```no_run\n")
        .expect("crate quick start")
        .1
        .lines()
        .take_while(|line| *line != "//! ```")
        .map(|line| {
            line.strip_prefix("//! ")
                .or_else(|| (line == "//!").then_some(""))
                .expect("quick-start lines are crate docs")
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(crate_docs, readme, "update the doctest from the README");
}
