//! Cross-server live conformance tests for papo.

#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    future::Future,
    path::Path,
    pin::Pin,
    process::{self, ExitCode},
    time::Duration,
};

use libtest_mimic::{Arguments, Completion, Conclusion, Failed, FormatSetting, Trial};
use papo::{Client, Endpoint, Event, Events, Isupport, isupport::Advertised};
use papo_core::{Capabilities, DEFAULT_WANTED_CAPS, Sasl};
use papo_test_server::{ServerInfo, from_env};
use tokio::{runtime::Builder, task::JoinHandle, time::timeout};

mod scenarios;

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

inventory::collect!(ScenarioSpec);

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
struct Scenario<S = ServerInfo> {
    server: S,
    name: &'static str,
}

struct Connection {
    client: Client,
    events: Events,
    driver: JoinHandle<Result<(), papo::Error>>,
}

struct Welcome {
    nick: String,
    caps: Capabilities,
    isupport: Isupport,
}

struct RegisteredClient {
    client: Client,
    events: Events,
    driver: JoinHandle<Result<(), papo::Error>>,
    welcome: Welcome,
}

trait EventSource {
    fn next_event(&mut self) -> impl Future<Output = Option<Event>>;
}

impl EventSource for Events {
    fn next_event(&mut self) -> impl Future<Output = Option<Event>> {
        self.next()
    }
}

/// Runs the live conformance test binary.
#[must_use]
pub fn run() -> ExitCode {
    let arguments = Arguments::from_args();
    let server = from_env();
    let specs = scenario_specs();
    let server_name = env::var("PAPO_TEST_PROFILE")
        .or_else(|_| env::var("PAPO_TEST_SERVER"))
        .unwrap_or_else(|_| "ergo-default".into());
    let known_failures =
        load_known_failures(&server_name, &specs).unwrap_or_else(|error| startup_error(&error));
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
        .map(|spec| {
            trial(
                spec,
                server.clone(),
                &probe,
                known_failures.get(spec.name).cloned(),
                server_name.clone(),
                arguments.format == Some(FormatSetting::Json),
            )
        })
        .collect();
    match run_trials(&arguments, trials) {
        Ok(conclusion) => conclusion.exit_code(),
        Err(error) => {
            eprintln!("conformance startup error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_trials(arguments: &Arguments, trials: Vec<Trial>) -> Result<Conclusion, String> {
    if trials.is_empty() {
        return Err("conformance suite contains no scenarios".into());
    }
    Ok(libtest_mimic::run(arguments, trials))
}

fn scenario_specs() -> Vec<ScenarioSpec> {
    scenarios::all()
}

fn trial(
    spec: ScenarioSpec,
    server: ServerInfo,
    probe: &Welcome,
    known_failure: Option<String>,
    server_name: String,
    json: bool,
) -> Trial {
    if let Some(reason) = spec.requires.unsupported(&probe.caps, &probe.isupport) {
        return Trial::ignorable_test(spec.name, move || {
            let result = Ok(Completion::ignored_with(reason));
            report_scenario_status(spec.name, &result, None, json);
            result
        })
        .with_kind(spec.doc);
    }
    Trial::ignorable_test(spec.name, move || {
        let result = runtime().block_on((spec.run)(Scenario {
            server,
            name: spec.name,
        }));
        report_scenario_status(spec.name, &result, known_failure.as_deref(), json);
        known_failure_verdict(result, known_failure.as_deref(), &server_name)
    })
    .with_kind(spec.doc)
}

fn report_scenario_status(
    name: &str,
    result: &ScenarioResult,
    known_failure: Option<&str>,
    json: bool,
) {
    if !json {
        return;
    }
    if let Some((event, reason)) = scenario_status(result, known_failure) {
        let reason = reason.map_or_else(|| "null".into(), json_string);
        println!(
            r#"{{ "type": "scenario", "name": {}, "event": {}, "reason": {} }}"#,
            json_string(name),
            json_string(event),
            reason
        );
    }
}

fn scenario_status<'a>(
    result: &'a ScenarioResult,
    known_failure: Option<&'a str>,
) -> Option<(&'static str, Option<&'a str>)> {
    match result {
        Ok(Completion::Ignored { reason }) => Some(("ignored", reason.as_deref())),
        Err(_) => known_failure.map(|reason| ("expected-failure", Some(reason))),
        Ok(Completion::Completed) => None,
    }
}

fn json_string(input: &str) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(input.len() + 2);
    output.push('"');
    for character in input.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character <= '\u{1f}' => {
                write!(output, "\\u{:04x}", character as u32)
                    .expect("writing to a String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

impl<S> Scenario<S> {
    fn unique(&self) -> String {
        papo_test_server::unique(self.name)
    }
}

impl Scenario {
    async fn client(&self, nick: &str) -> Result<RegisteredClient, Failed> {
        self.client_with(nick, None).await
    }

    async fn client_at(&self, nick: &str, endpoint: Endpoint) -> Result<RegisteredClient, Failed> {
        self.client_with_at(nick, endpoint, None).await
    }

    async fn client_caps(
        &self,
        nick: &str,
        extra_caps: &[&str],
    ) -> Result<RegisteredClient, Failed> {
        let endpoint = Endpoint::plaintext(
            self.server.plaintext_addr().ip().to_string(),
            self.server.plaintext_addr().port(),
        );
        let mut wanted_caps = DEFAULT_WANTED_CAPS
            .iter()
            .map(|cap| (*cap).to_owned())
            .collect::<Vec<_>>();
        wanted_caps.extend(extra_caps.iter().map(|cap| (*cap).to_owned()));
        let mut connection = connect_client(
            &self.server,
            endpoint,
            nick,
            "papo conformance client",
            Some(wanted_caps),
            None,
        )?;
        let welcome = wait_for_registration(&connection.client, &mut connection.events).await?;
        Ok(RegisteredClient {
            client: connection.client,
            events: connection.events,
            driver: connection.driver,
            welcome,
        })
    }

    async fn client_with(
        &self,
        nick: &str,
        sasl: Option<Sasl>,
    ) -> Result<RegisteredClient, Failed> {
        let endpoint = Endpoint::plaintext(
            self.server.plaintext_addr().ip().to_string(),
            self.server.plaintext_addr().port(),
        );
        self.client_with_at(nick, endpoint, sasl).await
    }

    async fn client_with_at(
        &self,
        nick: &str,
        endpoint: Endpoint,
        sasl: Option<Sasl>,
    ) -> Result<RegisteredClient, Failed> {
        let mut connection = connect_client(
            &self.server,
            endpoint,
            nick,
            "papo conformance client",
            None,
            sasl,
        )?;
        let welcome = wait_for_registration(&connection.client, &mut connection.events).await?;
        Ok(RegisteredClient {
            client: connection.client,
            events: connection.events,
            driver: connection.driver,
            welcome,
        })
    }

    async fn expect(
        &self,
        events: &mut Events,
        matcher: impl FnMut(&Event) -> bool,
        limit: Duration,
    ) -> Result<Event, Failed> {
        expect_event(events, matcher, limit).await
    }
}

async fn expect_event(
    events: &mut impl EventSource,
    mut matcher: impl FnMut(&Event) -> bool,
    limit: Duration,
) -> Result<Event, Failed> {
    timeout(limit, async {
        loop {
            match events.next_event().await {
                Some(event) if matcher(&event) => return Ok(event),
                Some(Event::Disconnected(disconnected)) => {
                    return Err(disconnected.to_string().into());
                }
                Some(_) => {}
                None => return Err("driver stopped while waiting for an event".into()),
            }
        }
    })
    .await
    .map_err(|_| Failed::from("timed out waiting for an event"))?
}

impl RegisteredClient {
    async fn close(self) -> ScenarioResult {
        self.client
            .quit("conformance scenario complete")
            .await
            .map_err(Failed::from)?;
        self.driver
            .await
            .map_err(Failed::from)?
            .map_err(Failed::from)?;
        Ok(Completion::Completed)
    }
}

fn load_known_failures(
    server: &str,
    scenarios: &[ScenarioSpec],
) -> Result<BTreeMap<String, String>, String> {
    load_known_failures_from(
        Path::new("known-failures"),
        server,
        scenarios.iter().map(|scenario| scenario.name),
    )
}

/// A missing file means that the server has no recorded conformance exceptions.
fn load_known_failures_from<'a>(
    directory: &Path,
    server: &str,
    scenarios: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeMap<String, String>, String> {
    if server.is_empty()
        || !server
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("invalid PAPO_TEST_SERVER value {server:?}"));
    }
    let path = directory.join(server).with_extension("toml");
    let input = match fs::read_to_string(&path) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    parse_known_failures(&input, scenarios)
}

fn parse_known_failures<'a>(
    input: &str,
    scenarios: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeMap<String, String>, String> {
    let table = input
        .parse::<toml::Table>()
        .map_err(|error| format!("invalid known-failures TOML: {error}"))?;
    let scenarios = scenarios.into_iter().collect::<BTreeSet<_>>();
    table
        .into_iter()
        .map(|(name, entry)| {
            if !scenarios.contains(name.as_str()) {
                return Err(format!("known failure names unknown scenario {name:?}"));
            }
            let reason = entry
                .as_table()
                .and_then(|entry| entry.get("reason"))
                .and_then(toml::Value::as_str)
                .filter(|reason| !reason.is_empty())
                .ok_or_else(|| format!("known failure {name:?} is missing a reason"))?;
            Ok((name, reason.to_owned()))
        })
        .collect()
}

fn known_failure_verdict(
    result: ScenarioResult,
    reason: Option<&str>,
    server: &str,
) -> ScenarioResult {
    let Some(reason) = reason else {
        return result;
    };
    match result {
        Err(_) => {
            eprintln!("expected failure: {reason}");
            Ok(Completion::Completed)
        }
        Ok(Completion::Completed) => {
            Err(format!("unexpected pass; remove it from known-failures/{server}.toml").into())
        }
        ignored @ Ok(Completion::Ignored { .. }) => ignored,
    }
}

async fn probe(
    server: &ServerInfo,
    wanted_caps: impl IntoIterator<Item = &str>,
) -> Result<Welcome, String> {
    let nick = papo_test_server::unique("papo-probe");
    let endpoint = Endpoint::plaintext(
        server.plaintext_addr().ip().to_string(),
        server.plaintext_addr().port(),
    );
    let wanted_caps = wanted_caps.into_iter().map(str::to_owned).collect();
    let mut connection = connect_client(
        server,
        endpoint,
        &nick,
        "papo conformance probe",
        Some(wanted_caps),
        None,
    )?;
    let welcome = wait_for_registration(&connection.client, &mut connection.events).await?;
    connection
        .client
        .quit("conformance probe complete")
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
    fn unsupported(&self, caps: &Capabilities, isupport: &impl IsupportView) -> Option<String> {
        for requirement in self.caps {
            let value = caps.get(requirement.name).or_else(|| {
                (requirement.name == "draft/metadata-2")
                    .then(|| caps.get("draft/metadata-3"))
                    .flatten()
            });
            let Some(value) = value else {
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
            matches!(isupport.advertised(token), Advertised::Absent)
                .then(|| format!("server does not advertise {token}"))
        })
    }
}

trait IsupportView {
    fn advertised(&self, token: &str) -> Advertised<'_>;
}

impl IsupportView for Isupport {
    fn advertised(&self, token: &str) -> Advertised<'_> {
        self.get(token)
    }
}

#[cfg(test)]
impl IsupportView for papo_proto::Isupport {
    fn advertised(&self, token: &str) -> Advertised<'_> {
        self.get(token)
    }
}

/// Reads setup supplied to a scenario.
///
/// # Errors
///
/// Returns a runtime-ignore completion when the named variable is unavailable.
pub fn required_environment(name: &str) -> Result<String, Completion> {
    required_environment_from(name, |name| env::var(name))
}

fn required_environment_from(
    name: &str,
    lookup: impl FnOnce(&str) -> Result<String, env::VarError>,
) -> Result<String, Completion> {
    match lookup(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) | Err(_) => Err(Completion::ignored_with(format!("{name} is not set up"))),
    }
}

fn connect_client(
    server: &ServerInfo,
    endpoint: Endpoint,
    nick: &str,
    realname: &str,
    wanted_caps: Option<Vec<String>>,
    sasl: Option<Sasl>,
) -> Result<Connection, String> {
    let ca_pem = std::fs::read(server.ca_pem_path()).map_err(|error| error.to_string())?;
    let mut builder = Client::builder(endpoint)
        .nick(nick)
        .username("papo")
        .realname(realname)
        .tls_ca_pem(ca_pem);
    if let Some(wanted_caps) = wanted_caps {
        builder = builder.request_caps(wanted_caps);
    }
    #[cfg(not(target_family = "wasm"))]
    let bouncer_login = server
        .bouncer()
        .map(|bouncer| bouncer.provision_network(nick))
        .transpose()
        .map_err(|error| error.to_string())?;
    #[cfg(not(target_family = "wasm"))]
    if let Some(login) = &bouncer_login {
        builder = builder.sasl(Sasl::plain(login.username(), login.password()));
    } else if let Some(sasl) = sasl {
        builder = builder.sasl(sasl);
    }
    #[cfg(target_family = "wasm")]
    if let Some(sasl) = sasl {
        builder = builder.sasl(sasl);
    }
    let (client, events, driver) = builder.build();
    Ok(Connection {
        client,
        events,
        driver: tokio::spawn(driver),
    })
}

async fn wait_for_registration(client: &Client, events: &mut Events) -> Result<Welcome, String> {
    timeout(TIMEOUT, async {
        loop {
            match events.next().await {
                Some(Event::Registered(registered)) => {
                    return Ok(Welcome {
                        nick: registered.nick.into_string(),
                        caps: registered.caps,
                        isupport: client.state().isupport().clone(),
                    });
                }
                Some(Event::Disconnected(disconnected)) => {
                    return Err(disconnected.to_string());
                }
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
    use std::{
        collections::VecDeque,
        fs, future,
        path::{Path, PathBuf},
        time::Duration,
    };

    use papo_core::Capabilities;
    use papo_proto::Isupport;

    use libtest_mimic::{Arguments, Completion, Failed, Trial};

    use super::{
        CapRequirement, Event, EventSource, Requirements, Scenario, expect_event, json_string,
        known_failure_verdict, load_known_failures_from, required_environment_from, run_trials,
        runtime, scenario_status,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create() -> Self {
            let path = Path::new("target")
                .join("papo-conformance-tests")
                .join(papo_test_server::unique("known-failures"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, input: &str) {
            fs::write(self.0.join("ergo.toml"), input).unwrap();
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    struct ScriptedEvents {
        events: VecDeque<Event>,
    }

    impl EventSource for ScriptedEvents {
        fn next_event(&mut self) -> impl Future<Output = Option<Event>> {
            let event = self.events.pop_front();
            async move {
                match event {
                    Some(event) => Some(event),
                    None => future::pending().await,
                }
            }
        }
    }

    #[test]
    fn expect_skips_non_matching_events() {
        let mut events = ScriptedEvents {
            events: [
                Event::AccountRegistration(papo::AccountRegistration::Registered {
                    account: "first".into(),
                }),
                Event::AccountRegistration(papo::AccountRegistration::Registered {
                    account: "second".into(),
                }),
            ]
            .into(),
        };
        let event = runtime()
            .block_on(expect_event(
                &mut events,
                |event| matches!(event, Event::AccountRegistration(papo::AccountRegistration::Registered { account }) if account == "second"),
                Duration::from_secs(1),
            ))
            .unwrap();
        assert!(
            matches!(event, Event::AccountRegistration(papo::AccountRegistration::Registered { account }) if account == "second")
        );
    }

    #[test]
    fn expect_times_out_after_only_non_matching_events() {
        let mut events = ScriptedEvents {
            events: [Event::Warning(papo::event::Warning::MonitorUnsupported)].into(),
        };
        let error = runtime()
            .block_on(expect_event(
                &mut events,
                |_| false,
                Duration::from_millis(1),
            ))
            .err()
            .unwrap();
        assert_eq!(error.message(), Some("timed out waiting for an event"));
    }

    #[test]
    fn harness_rejects_an_empty_trial_list() {
        let arguments = Arguments::from_iter(["papo-conformance", "--test-threads=1"]);
        assert_eq!(
            run_trials(&arguments, Vec::new()).unwrap_err(),
            "conformance suite contains no scenarios"
        );
    }

    #[test]
    fn harness_executes_failing_trials() {
        let arguments = Arguments::from_iter(["papo-conformance", "--test-threads=1"]);
        let trials = vec![Trial::test("deliberate_failure", || {
            Err("deliberate harness failure".into())
        })];
        let conclusion = run_trials(&arguments, trials).unwrap();
        assert!(conclusion.has_failed());
        assert_eq!(conclusion.num_failed, 1);
        assert_eq!(conclusion.num_passed, 0);
    }

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

    #[test]
    fn known_failure_loader_reads_and_validates_real_files() {
        let directory = TestDirectory::create();
        let scenarios = ["registration", "ping"];

        let missing = load_known_failures_from(&directory.0, "ergo", scenarios).unwrap();
        assert!(missing.is_empty());
        assert!(load_known_failures_from(&directory.0, "", scenarios).is_err());
        assert!(load_known_failures_from(&directory.0, "../ergo", scenarios).is_err());

        directory
            .write("[registration]\nreason = 'server bug 417'\n[ping]\nreason = 'missing PONG'\n");
        let parsed = load_known_failures_from(&directory.0, "ergo", scenarios).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["registration"], "server bug 417");
        assert_eq!(parsed["ping"], "missing PONG");

        for (input, expected) in [
            ("[registration]\nnote = 'missing'\n", "missing a reason"),
            ("[registration]\nreason = ''\n", "missing a reason"),
            ("[ghost]\nreason = 'stale'\n", "unknown scenario"),
            (
                "[registration\nreason = 'broken'\n",
                "invalid known-failures TOML",
            ),
        ] {
            directory.write(input);
            let error = load_known_failures_from(&directory.0, "ergo", scenarios).unwrap_err();
            assert!(error.contains(expected), "{error:?}");
        }
    }

    #[test]
    fn known_failure_verdicts_invert_failure_and_pass() {
        let expected = known_failure_verdict(
            Err(Failed::from("server rejected command")),
            Some("server bug 417"),
            "ergo",
        );
        assert!(matches!(expected, Ok(Completion::Completed)));

        let unexpected =
            known_failure_verdict(Ok(Completion::Completed), Some("server bug 417"), "ergo");
        let Err(unexpected) = unexpected else {
            panic!("a known failure passing must fail the trial")
        };
        assert_eq!(
            unexpected.message(),
            Some("unexpected pass; remove it from known-failures/ergo.toml")
        );
    }

    #[test]
    fn scenario_status_records_ignored_and_expected_failures() {
        let ignored = Ok(Completion::ignored_with("PAPO_TEST_WS is not set up"));
        assert_eq!(
            scenario_status(&ignored, None),
            Some(("ignored", Some("PAPO_TEST_WS is not set up")))
        );
        let failed = Err(Failed::from("server rejected METADATA"));
        assert_eq!(
            scenario_status(&failed, Some("server bug 417")),
            Some(("expected-failure", Some("server bug 417")))
        );
        assert_eq!(scenario_status(&failed, None), None);
        assert_eq!(
            json_string("quote \" and\ncontrol\u{1f}"),
            "\"quote \\\" and\\ncontrol\\u001f\""
        );
    }

    #[test]
    fn required_environment_distinguishes_set_and_unset_values() {
        let configured = required_environment_from("PAPO_TEST_OPER", |name| {
            assert_eq!(name, "PAPO_TEST_OPER");
            Ok("oper-name:correct horse battery staple".into())
        });
        let Ok(configured) = configured else {
            panic!("a configured setup variable must return its value")
        };
        assert_eq!(configured, "oper-name:correct horse battery staple");

        let missing =
            required_environment_from("PAPO_TEST_OPER", |_| Err(std::env::VarError::NotPresent));
        match missing {
            Err(Completion::Ignored {
                reason: Some(reason),
            }) => assert_eq!(reason, "PAPO_TEST_OPER is not set up"),
            _ => panic!("an unset setup variable must ignore the scenario with a reason"),
        }
        let empty = required_environment_from("PAPO_TEST_WS", |_| Ok(String::new()));
        assert!(matches!(empty, Err(Completion::Ignored { .. })));
    }

    #[test]
    fn scenario_unique_names_keep_the_prefix_and_differ() {
        let scenario = Scenario {
            server: (),
            name: "registration",
        };
        let first = scenario.unique();
        let second = scenario.unique();
        assert!(first.starts_with("registration-"), "{first:?}");
        assert!(second.starts_with("registration-"), "{second:?}");
        assert_ne!(first, second);
    }
}
