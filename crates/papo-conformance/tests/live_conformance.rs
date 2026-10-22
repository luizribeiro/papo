//! Live cross-server conformance tests.

use papo_conformance::{
    CapRequirement, Failure, Peer, Requirements, Scenario, ScenarioFuture, ScenarioSpec,
    run_scenario, source_nick,
};

#[path = "live_conformance/scenarios/mod.rs"]
mod scenarios;
