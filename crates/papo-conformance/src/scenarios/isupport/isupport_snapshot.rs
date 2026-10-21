use std::collections::BTreeSet;
#[cfg(not(target_family = "wasm"))]
use std::{env, fs};

use papo::isupport::Advertised;
use papo_conformance_macros::scenario;
use papo_test_server::DeploymentProfile;

use crate::Scenario;

/// Keys whose advertised value is known to vary between runs of the same profile.
///
/// `BOUNCER_NETID` is soju's per-session network id. `VAPID` also varies on soju-ergo: each
/// test run starts soju against a fresh, unpersisted web-push keystore, so it generates a new
/// VAPID keypair every time rather than reusing one across runs.
const VARYING: &[&str] = &["BOUNCER_NETID", "VAPID"];

fn render_line(key: &str, value: Advertised<'_>) -> String {
    if VARYING.contains(&key) {
        return format!("{key}=<varies>");
    }
    match value {
        Advertised::Value(value) => format!("{key}={value}"),
        Advertised::Bare => key.to_owned(),
        Advertised::Absent => unreachable!("Isupport::tokens only yields advertised keys"),
    }
}

fn render_snapshot<'a>(tokens: impl Iterator<Item = (&'a str, Advertised<'a>)>) -> String {
    let mut lines: Vec<String> = tokens.map(|(key, value)| render_line(key, value)).collect();
    lines.sort_unstable();
    let mut snapshot = lines.join("\n");
    snapshot.push('\n');
    snapshot
}

fn is_clean_snapshot_line(line: &str) -> bool {
    !line.contains(' ') && !line.contains('\r')
}

fn expected_snapshot(profile: DeploymentProfile) -> &'static str {
    match profile {
        DeploymentProfile::ErgoDefault => {
            include_str!("../../../snapshots/isupport/ergo-default.txt")
        }
        DeploymentProfile::ErgoPersistent => {
            include_str!("../../../snapshots/isupport/ergo-persistent.txt")
        }
        DeploymentProfile::SojuErgo => include_str!("../../../snapshots/isupport/soju-ergo.txt"),
        DeploymentProfile::InspIrcd => include_str!("../../../snapshots/isupport/inspircd.txt"),
        DeploymentProfile::Solanum => include_str!("../../../snapshots/isupport/solanum.txt"),
        DeploymentProfile::UnrealIrcd => include_str!("../../../snapshots/isupport/unrealircd.txt"),
    }
}

fn snapshot_diff(expected: &str, actual: &str) -> String {
    let expected_lines: BTreeSet<&str> = expected.lines().collect();
    let actual_lines: BTreeSet<&str> = actual.lines().collect();
    let mut diff = String::new();
    for line in &expected_lines {
        if !actual_lines.contains(line) {
            diff.push_str("- ");
            diff.push_str(line);
            diff.push('\n');
        }
    }
    for line in &actual_lines {
        if !expected_lines.contains(line) {
            diff.push_str("+ ");
            diff.push_str(line);
            diff.push('\n');
        }
    }
    diff
}

/// the advertised ISUPPORT set matches the checked-in snapshot for this profile
#[scenario(covers = ["base.isupport.snapshot"])]
async fn isupport_snapshot(s: Scenario) -> papo_testing::Result<()> {
    let alice = s
        .connect_with(s.configure("alice")?.enable(papo::Feature::WebPush))
        .await?;
    let state = alice.state();
    let support = state.isupport();
    let actual = render_snapshot(support.tokens());
    assert!(
        actual.lines().all(is_clean_snapshot_line),
        "an ISUPPORT snapshot line contains a space or carriage return: {actual:?}"
    );

    let profile = DeploymentProfile::from_env()?;

    #[cfg(not(target_family = "wasm"))]
    {
        if env::var("PAPO_BLESS_ISUPPORT").is_ok_and(|value| value == "1") {
            let path = format!(
                "{}{}.txt",
                concat!(env!("CARGO_MANIFEST_DIR"), "/snapshots/isupport/"),
                profile.as_str()
            );
            fs::write(path, &actual)?;
            return Ok(());
        }
    }

    let expected = expected_snapshot(profile);
    if actual != expected {
        let diff = snapshot_diff(expected, &actual);
        return Err(format!(
            "ISUPPORT snapshot mismatch for profile {profile}:\n{diff}re-run with PAPO_BLESS_ISUPPORT=1 to accept"
        )
        .into());
    }
    assert_eq!(actual.lines().count(), support.tokens().len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use papo::isupport::Advertised;

    use super::{VARYING, is_clean_snapshot_line, render_snapshot, snapshot_diff};

    #[test]
    fn renders_sorted_bare_and_valued_tokens_with_a_trailing_newline() {
        let tokens = [
            ("NETWORK", Advertised::Value("ErgoTest")),
            ("SAFELIST", Advertised::Bare),
            ("BOT", Advertised::Value("B")),
            ("AWAYLEN", Advertised::Value("390")),
        ];
        assert_eq!(
            render_snapshot(tokens.into_iter()),
            "AWAYLEN=390\nBOT=B\nNETWORK=ErgoTest\nSAFELIST\n"
        );
    }

    #[test]
    fn renders_an_empty_value_with_a_trailing_equals_sign() {
        let tokens = [("EXTBAN", Advertised::Value(""))];
        assert_eq!(render_snapshot(tokens.into_iter()), "EXTBAN=\n");
    }

    #[test]
    fn normalizes_varying_keys_regardless_of_their_advertised_value() {
        assert_eq!(VARYING, ["BOUNCER_NETID", "VAPID"].as_slice());
        let tokens = [
            ("BOUNCER_NETID", Advertised::Value("soju-session-7f3a")),
            (
                "VAPID",
                Advertised::Value("BI_E_9VJ-Ie37dZRG8MvwMq-XyHtHvCpmzqKrzKINzQeO"),
            ),
        ];
        assert_eq!(
            render_snapshot(tokens.into_iter()),
            "BOUNCER_NETID=<varies>\nVAPID=<varies>\n"
        );
    }

    #[test]
    fn diffs_an_added_a_removed_and_a_changed_token() {
        let expected = "CHANTYPES=#\nFORWARD=f\nNETWORK=ErgoTest\n";
        let actual = "CHANTYPES=#\nNETWORK=ErgoLive\nSAFERATE\n";
        assert_eq!(
            snapshot_diff(expected, actual),
            "- FORWARD=f\n- NETWORK=ErgoTest\n+ NETWORK=ErgoLive\n+ SAFERATE\n"
        );
    }

    #[test]
    fn diff_of_identical_snapshots_is_empty() {
        let snapshot = "CHANTYPES=#\nNETWORK=ErgoTest\n";
        assert_eq!(snapshot_diff(snapshot, snapshot), "");
    }

    #[test]
    fn rejects_lines_containing_a_space_or_carriage_return() {
        assert!(is_clean_snapshot_line("CHANTYPES=#"));
        assert!(!is_clean_snapshot_line("NETWORK=Ergo Test"));
        assert!(!is_clean_snapshot_line("NETWORK=ErgoTest\r"));
    }
}
