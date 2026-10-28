//! Guards the single version source used by the published crates.

const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const PAPO_MANIFEST: &str = include_str!("../Cargo.toml");
const CORE_MANIFEST: &str = include_str!("../../papo-core/Cargo.toml");
const PROTO_MANIFEST: &str = include_str!("../../papo-proto/Cargo.toml");

fn section<'a>(manifest: &'a str, heading: &str) -> Vec<&'a str> {
    manifest
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .collect()
}

fn quoted_value<'a>(lines: &[&'a str], key: &str) -> Option<&'a str> {
    let prefix = format!("{key} = \"");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix)?.strip_suffix('"'))
}

fn assert_workspace_version(package: &str, manifest: &str) {
    assert!(
        section(manifest, "[package]").contains(&"version.workspace = true"),
        "{package} must inherit the workspace version"
    );
}

fn assert_exact_requirement(package: &str, manifest: &str, dependency: &str, version: &str) {
    let prefix = format!("{dependency} = ");
    let requirement = format!("version = \"={version}\"");
    let dependencies = section(manifest, "[dependencies]");
    let line = dependencies
        .iter()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("{package} must depend on {dependency}"));
    assert!(
        line.contains(&requirement),
        "{package} must require {dependency} at exactly {version}"
    );
}

#[test]
fn published_crates_share_one_exact_version() {
    let workspace = section(WORKSPACE_MANIFEST, "[workspace.package]");
    let version = quoted_value(&workspace, "version").expect("workspace version must be a string");
    assert_eq!(version, env!("CARGO_PKG_VERSION"));

    for (package, manifest) in [
        ("papo", PAPO_MANIFEST),
        ("papo-core", CORE_MANIFEST),
        ("papo-proto", PROTO_MANIFEST),
    ] {
        assert_workspace_version(package, manifest);
    }

    assert_exact_requirement("papo", PAPO_MANIFEST, "papo-core", version);
    assert_exact_requirement("papo", PAPO_MANIFEST, "papo-proto", version);
    assert_exact_requirement("papo-core", CORE_MANIFEST, "papo-proto", version);
}
