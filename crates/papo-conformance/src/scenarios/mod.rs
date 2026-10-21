use super::ScenarioSpec;

mod auth;
mod connection;
mod history;
mod isupport;
mod line_length;
mod membership;
mod messaging;
mod metadata;
mod presence;
mod requests;
mod support;

pub(super) fn all() -> Vec<ScenarioSpec> {
    let mut scenarios = inventory::iter::<ScenarioSpec>
        .into_iter()
        .copied()
        .collect::<Vec<_>>();
    scenarios.sort_unstable_by_key(|scenario| scenario.name);
    scenarios
}
