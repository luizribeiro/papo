//! Checks that facade types have one canonical public path.

use std::collections::{BTreeMap, BTreeSet};

const STABLE: &str = include_str!("../public-api.txt");
const UNSTABLE: &str = include_str!("../public-api-unstable.txt");
const BOT: &str = include_str!("../../papo-bot/public-api.txt");

fn public_type_path(line: &str) -> Option<&str> {
    let public = line.find("pub ")?;
    let declaration = &line[public + 4..];
    let path = if let Some(path) = declaration.strip_prefix("use ") {
        path
    } else {
        ["struct ", "enum ", "union ", "trait ", "type "]
            .into_iter()
            .find_map(|kind| declaration.strip_prefix(kind))?
    };
    let path = path
        .split(|character: char| !(character.is_ascii_alphanumeric() || "_:".contains(character)))
        .next()?
        .trim_end_matches(':');
    let segments: Vec<_> = path.split("::").collect();
    let name = *segments.last()?;
    if !name.chars().next().is_some_and(char::is_uppercase)
        || segments[1..segments.len() - 1]
            .iter()
            .any(|segment| segment.starts_with(char::is_uppercase))
    {
        return None;
    }
    Some(path)
}

fn duplicate_type_paths(snapshot: &str) -> Vec<Vec<&str>> {
    let mut paths = BTreeMap::<_, BTreeSet<_>>::new();
    for path in snapshot.lines().filter_map(public_type_path) {
        let name = path.rsplit("::").next().expect("a public path has a name");
        paths.entry(name).or_default().insert(path);
    }
    paths
        .into_values()
        .filter(|paths| paths.len() > 1)
        .filter(|paths| {
            paths.iter().copied().collect::<Vec<_>>() != ["papo::Extban", "papo::isupport::Extban"]
        })
        .map(|paths| paths.into_iter().collect())
        .collect()
}

#[test]
fn public_types_have_one_path() {
    for (name, snapshot) in [
        ("papo", STABLE),
        ("papo unstable", UNSTABLE),
        ("papo-bot", BOT),
    ] {
        assert_eq!(
            duplicate_type_paths(snapshot),
            Vec::<Vec<&str>>::new(),
            "{name} exposes a type through more than one public path"
        );
    }
}

#[test]
fn path_check_finds_root_and_module_reexports() {
    let snapshot = "pub use example::Thing\n#[non_exhaustive] pub struct example::event::Thing\n";
    assert_eq!(
        duplicate_type_paths(snapshot),
        vec![vec!["example::Thing", "example::event::Thing"]]
    );
}
