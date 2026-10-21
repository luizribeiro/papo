//! Guards the capability registry as the sole source of protocol spellings.

#![cfg(not(target_os = "wasi"))]

use std::path::{Path, PathBuf};

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn is_test_source(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("tests.rs" | "test_support.rs" | "cap.rs")
    )
}

fn is_protocol_tag(path: &Path, line: &str, name: &str) -> bool {
    name == papo_core::cap::BATCH.as_str()
        && path.file_name().is_some_and(|name| name == "batch.rs")
        && (line.contains(".tag(")
            || line.contains("with_tag(")
            || line.contains("key ==")
            || line.contains("matches!(*key"))
}

#[test]
fn capability_literals_live_only_in_the_registry() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates = core.parent().unwrap();
    let mut files = Vec::new();
    for package in ["papo-core", "papo-proto", "papo"] {
        rust_files(&crates.join(package).join("src"), &mut files);
    }

    let mut failures = Vec::new();
    for path in files.into_iter().filter(|path| !is_test_source(path)) {
        let source = std::fs::read_to_string(&path).unwrap();
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//")
                || trimmed.starts_with("mod tests {")
                || trimmed.starts_with("#[tokio::test")
            {
                if !trimmed.starts_with("//") {
                    break;
                }
                continue;
            }
            for info in papo_core::cap::ALL {
                for name in std::iter::once(info.name).chain(info.aliases.iter().copied()) {
                    if line.contains(&format!("\"{name}\"")) && !is_protocol_tag(&path, line, name)
                    {
                        failures.push(format!("{}:{}: {name}", path.display(), index + 1));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "capability literals outside cap.rs:\n{}",
        failures.join("\n")
    );
}
