use std::{env, iter, sync::OnceLock};

use papo_core::cap::Request;

use super::{
    FixtureResult, Scenario, ScenarioSpec, Welcome, catch_scenario_panic, json_string,
    load_known_failures, probe, runtime, scenario,
};

static CONTEXT: OnceLock<Result<Context, Failure>> = OnceLock::new();

struct Context {
    server: papo_test_server::ServerInfo,
    profile: String,
    known_failures: std::collections::BTreeMap<String, String>,
    welcome: Welcome,
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub(super) struct Failure(String);

impl Context {
    fn load() -> Result<Self, Failure> {
        let server = papo_test_server::from_env();
        let profile = env::var("PAPO_TEST_PROFILE")
            .or_else(|_| env::var("PAPO_TEST_SERVER"))
            .unwrap_or_else(|_| "ergo-default".into());
        let known_failures = load_known_failures(&profile).map_err(Failure)?;
        let wanted_caps = papo_core::cap::ALL
            .iter()
            .filter(|info| !matches!(info.request, Request::Never))
            .flat_map(|info| iter::once(info.name).chain(info.aliases.iter().copied()));
        let welcome = runtime()
            .block_on(probe(&server, wanted_caps))
            .map_err(Failure)?;
        Ok(Self {
            server,
            profile,
            known_failures,
            welcome,
        })
    }
}

#[doc(hidden)]
pub(super) fn run_scenario(spec: &ScenarioSpec) -> Result<(), Failure> {
    if env::var_os("PAPO_CONFORMANCE_DESCRIBE").is_some() {
        println!(
            r#"{{ "type": "covers", "name": {}, "behaviors": [{}] }}"#,
            json_string(spec.name),
            spec.covers
                .iter()
                .map(|behavior| json_string(behavior))
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(());
    }
    let context = CONTEXT
        .get_or_init(Context::load)
        .as_ref()
        .map_err(Clone::clone)?;
    if let Some(reason) = spec
        .requires
        .unsupported(&context.welcome.caps, &context.welcome.isupport)
    {
        emit_status(spec.name, "ignored", Some(&reason));
        return Ok(());
    }
    let result = catch_scenario_panic(|| {
        runtime().block_on((spec.run)(Scenario {
            fixture: Some(scenario::Scenario::new(context.server.clone())),
            server: context.server.clone(),
            name: spec.name,
            json: true,
        }))
    });
    finish(
        spec.name,
        result,
        context.known_failures.get(spec.name),
        &context.profile,
    )
}

fn finish(
    name: &str,
    result: FixtureResult,
    known_failure: Option<&String>,
    profile: &str,
) -> Result<(), Failure> {
    if let Err(error) = &result
        && let Some(reason) = papo_testing::skip_reason(error)
    {
        emit_status(name, "ignored", Some(reason));
        return Ok(());
    }
    match (result, known_failure) {
        (Ok(()), None) => emit_status(name, "passed", None),
        (Err(error), None) => {
            emit_status(name, "failed", None);
            return Err(Failure(error.to_string()));
        }
        (Err(_), Some(reason)) => emit_status(name, "expected-failure", Some(reason)),
        (Ok(()), Some(reason)) => {
            emit_status(name, "unexpected-pass", Some(reason));
            return Err(Failure(format!(
                "unexpected pass; remove it from known-failures/{profile}.toml"
            )));
        }
    }
    Ok(())
}

fn emit_status(name: &str, event: &str, reason: Option<&str>) {
    let reason = reason.map_or_else(|| "null".into(), json_string);
    println!(
        r#"{{ "type": "scenario", "name": {}, "event": {}, "reason": {} }}"#,
        json_string(name),
        json_string(event),
        reason
    );
}

#[cfg(test)]
mod tests {
    use super::finish;

    #[test]
    fn known_failures_invert_failures_and_passes() {
        let reason = "server returns 482 rather than 441".to_owned();
        assert!(finish("kick", Ok(()), None, "ergo-default").is_ok());
        assert!(
            finish(
                "kick",
                Err(std::io::Error::other("482").into()),
                Some(&reason),
                "ergo-default"
            )
            .is_ok()
        );
        assert!(finish("kick", Ok(()), Some(&reason), "ergo-default").is_err());
    }
}
