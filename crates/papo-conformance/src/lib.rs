//! Cross-server live conformance tests for papo.

#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::{env, future::Future, pin::Pin, process, time::Duration};

use libtest_mimic::{Arguments, Completion, Failed, Trial};
use papo::{Client, Driver, Event, Events, OsEntropy, transport};
use papo_core::{Capabilities, Config, Session, Welcome};
use papo_proto::{Advertised, Isupport};
use papo_test_server::{ServerInfo, from_env};
use tokio::{runtime::Builder, task::JoinHandle, time::timeout};

const TIMEOUT: Duration = Duration::from_secs(10);

type ScenarioResult = Result<Completion, Failed>;
type ScenarioFuture = Pin<Box<dyn Future<Output = ScenarioResult>>>;

#[derive(Clone, Copy)]
struct ScenarioSpec {
    name: &'static str,
    doc: &'static str,
    requires: Requirements,
    run: fn(Scenario) -> ScenarioFuture,
}

#[derive(Clone, Copy, Default)]
struct Requirements {
    caps: &'static [CapRequirement],
    isupport: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct CapRequirement {
    name: &'static str,
    value: Option<&'static str>,
}

#[derive(Clone)]
struct Scenario {
    _server: ServerInfo,
}

struct Connection {
    client: Client,
    events: Events,
    driver: JoinHandle<Result<(), papo::Error>>,
}

/// Runs the live conformance test binary.
pub fn run() {
    let arguments = Arguments::from_args();
    let server = from_env();
    let specs = scenario_specs();
    let wanted_caps = specs
        .iter()
        .flat_map(|spec| spec.requires.caps)
        .map(|requirement| requirement.name);
    let probe = runtime()
        .block_on(probe(&server, wanted_caps))
        .unwrap_or_else(|error| startup_error(&error));
    let trials = specs
        .iter()
        .copied()
        .map(|spec| trial(spec, server.clone(), &probe))
        .collect();
    libtest_mimic::run(&arguments, trials).exit();
}

fn scenario_specs() -> &'static [ScenarioSpec] {
    &[]
}

fn trial(spec: ScenarioSpec, server: ServerInfo, probe: &Welcome) -> Trial {
    if let Some(reason) = spec.requires.unsupported(&probe.caps, &probe.isupport) {
        return Trial::ignorable_test(spec.name, move || Ok(Completion::ignored_with(reason)))
            .with_kind(spec.doc);
    }
    Trial::ignorable_test(spec.name, move || {
        runtime().block_on((spec.run)(Scenario { _server: server }))
    })
    .with_kind(spec.doc)
}

async fn probe(
    server: &ServerInfo,
    wanted_caps: impl IntoIterator<Item = &str>,
) -> Result<Welcome, String> {
    let nick = papo_test_server::unique("papo-probe");
    let config = Config::default()
        .nick(&nick)
        .username("papo")
        .realname("papo conformance probe")
        .wanted_caps(wanted_caps);
    let mut connection = connect_client(server, config).await?;
    let welcome = wait_for_registration(&mut connection.events).await?;
    connection
        .client
        .quit(Some("conformance probe complete"))
        .await
        .map_err(|error| error.to_string())?;
    connection
        .driver
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    Ok(welcome)
}

impl Requirements {
    fn unsupported(&self, caps: &Capabilities, isupport: &Isupport) -> Option<String> {
        for requirement in self.caps {
            let Some(value) = caps.get(requirement.name) else {
                return Some(format!("server does not advertise {}", requirement.name));
            };
            if let Some(required) = requirement.value
                && !value.as_deref().is_some_and(|advertised| {
                    advertised
                        .split(',')
                        .any(|value| value.eq_ignore_ascii_case(required))
                })
            {
                return Some(format!(
                    "server does not advertise {}={required}",
                    requirement.name
                ));
            }
        }
        self.isupport.iter().find_map(|token| {
            matches!(isupport.get(token), Advertised::Absent)
                .then(|| format!("server does not advertise {token}"))
        })
    }
}

/// Reads setup supplied to a scenario.
///
/// # Errors
///
/// Returns a runtime-ignore completion when the named variable is unavailable.
pub fn required_environment(name: &str) -> Result<String, Completion> {
    env::var(name).map_err(|_| Completion::ignored_with(format!("{name} is not set up")))
}

async fn connect_client(server: &ServerInfo, config: Config) -> Result<Connection, String> {
    let address = server.plaintext_addr();
    let stream = transport::connect(
        &address.ip().to_string(),
        address.port(),
        &transport::Security::Plaintext,
    )
    .await
    .map_err(|error| error.to_string())?;
    let session = Session::with_entropy(config.host(address.ip().to_string()), Box::new(OsEntropy));
    let (client, events, driver) = Driver::new(stream, session);
    Ok(Connection {
        client,
        events,
        driver: tokio::spawn(driver),
    })
}

async fn wait_for_registration(events: &mut Events) -> Result<Welcome, String> {
    timeout(TIMEOUT, async {
        loop {
            match events.next().await {
                Some(Event::Registered(welcome)) => return Ok(welcome),
                Some(Event::Disconnected { reason }) => return Err(reason),
                Some(_) => {}
                None => return Err("driver stopped during registration".into()),
            }
        }
    })
    .await
    .map_err(|_| "registration timed out".to_owned())?
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread tokio runtime")
}

fn startup_error(error: &str) -> ! {
    eprintln!("conformance startup error: {error}");
    process::exit(1)
}

#[cfg(test)]
mod tests {
    use papo_core::Capabilities;
    use papo_proto::Isupport;

    use super::{CapRequirement, Requirements};

    #[test]
    fn requirements_match_cap_names_values_and_isupport_tokens() {
        let mut caps = Capabilities::new();
        caps.insert("sasl".into(), Some("PLAIN,SCRAM-SHA-256".into()));
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CASEMAPPING=ascii"]);
        let requirements = Requirements {
            caps: &[CapRequirement {
                name: "sasl",
                value: Some("scram-sha-256"),
            }],
            isupport: &["casemapping"],
        };
        assert_eq!(requirements.unsupported(&caps, &isupport), None);

        caps.clear();
        assert_eq!(
            requirements.unsupported(&caps, &isupport).as_deref(),
            Some("server does not advertise sasl")
        );
    }

    #[test]
    fn requirements_reject_missing_values_and_tokens() {
        let mut caps = Capabilities::new();
        caps.insert("sasl".into(), Some("PLAIN".into()));
        let requirements = Requirements {
            caps: &[CapRequirement {
                name: "sasl",
                value: Some("SCRAM-SHA-256"),
            }],
            isupport: &["CHATHISTORY"],
        };
        assert_eq!(
            requirements
                .unsupported(&caps, &Isupport::default())
                .as_deref(),
            Some("server does not advertise sasl=SCRAM-SHA-256")
        );
        caps.insert("sasl".into(), Some("SCRAM-SHA-256".into()));
        assert_eq!(
            requirements
                .unsupported(&caps, &Isupport::default())
                .as_deref(),
            Some("server does not advertise CHATHISTORY")
        );
    }
}
