//! Emits scenario-to-checklist coverage as tab-separated rows.

fn main() {
    for coverage in papo_conformance::scenario_coverage() {
        if coverage.behaviors.is_empty() {
            println!("{}\t", coverage.scenario);
        }
        for behavior in coverage.behaviors {
            println!("{}\t{behavior}", coverage.scenario);
        }
    }
}
