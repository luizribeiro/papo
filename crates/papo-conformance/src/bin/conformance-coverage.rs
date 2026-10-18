//! Emits scenario-to-checklist coverage as tab-separated rows.

fn main() {
    for coverage in papo_conformance::scenario_coverage() {
        for behavior in coverage.behaviors {
            println!("{}\t{behavior}", coverage.scenario);
        }
    }
}
