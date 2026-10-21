//! Cross-server live conformance tests for papo.

#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    ops::Deref,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    pin::Pin,
    time::Duration,
};

use papo::{Client, Endpoint, Event, Events, Isupport, isupport::Advertised};
use papo_core::{Capabilities, Sasl};
use papo_test_server::ServerInfo;
use tokio::{runtime::Builder, task::JoinHandle, time::timeout};

mod peer;
mod runner;
mod scenario;

#[doc(hidden)]
pub use peer::Peer;
#[doc(hidden)]
pub use runner::{Failure, run_scenario};

const TIMEOUT: Duration = Duration::from_secs(10);

type FixtureResult = papo_testing::Result<()>;
#[doc(hidden)]
pub type ScenarioFuture = Pin<Box<dyn Future<Output = FixtureResult>>>;

#[doc(hidden)]
pub fn source_nick(source: &papo::Source) -> Option<&str> {
    source.nick().map(AsRef::<str>::as_ref)
}

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct ScenarioSpec {
    pub name: &'static str,
    pub requires: Requirements,
    pub covers: &'static [&'static str],
    pub run: fn(Scenario) -> ScenarioFuture,
}

#[derive(Clone, Copy, Default)]
#[doc(hidden)]
pub struct Requirements {
    pub caps: &'static [CapRequirement],
    pub isupport: &'static [&'static str],
}

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct CapRequirement {
    pub name: &'static str,
    pub value: Option<&'static str>,
}

#[derive(Clone)]
#[doc(hidden)]
pub struct Scenario<S = ServerInfo> {
    pub server: S,
    name: &'static str,
    fixture: Option<scenario::Scenario>,
    json: bool,
}

struct Connection {
    client: Client,
    events: Events,
    driver: JoinHandle<Result<(), papo::Error>>,
}

struct Welcome {
    caps: Capabilities,
    isupport: Isupport,
}

fn catch_scenario_panic(run: impl FnOnce() -> FixtureResult) -> FixtureResult {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic");
            Err(std::io::Error::other(format!("scenario panicked: {message}")).into())
        }
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

impl Scenario {
    #[doc(hidden)]
    pub fn record_evidence(&self, value: &str) {
        if self.json {
            println!(
                r#"{{ "type": "evidence", "name": {}, "value": {} }}"#,
                json_string(self.name),
                json_string(value)
            );
        }
    }
}

impl Deref for Scenario {
    type Target = scenario::Scenario;

    fn deref(&self) -> &Self::Target {
        self.fixture
            .as_ref()
            .expect("live scenarios have a fixture")
    }
}

fn load_known_failures(server: &str) -> Result<BTreeMap<String, String>, String> {
    load_known_failures_from(Path::new("known-failures"), server)
}

/// A missing file means that the server has no recorded conformance exceptions.
fn load_known_failures_from(
    directory: &Path,
    server: &str,
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
    parse_known_failures(&input)
}

fn parse_known_failures(input: &str) -> Result<BTreeMap<String, String>, String> {
    let table = input
        .parse::<toml::Table>()
        .map_err(|error| format!("invalid known-failures TOML: {error}"))?;
    table
        .into_iter()
        .map(|(name, entry)| {
            let entry = entry
                .as_table()
                .ok_or_else(|| format!("known failure {name:?} must be a table"))?;
            let reason = known_failure_field(entry, &name, "reason")?;
            let kind = known_failure_field(entry, &name, "kind")?;
            if !matches!(
                kind,
                "server-bug" | "not-implemented" | "spec-ambiguity" | "fixture-limit"
            ) {
                return Err(format!("known failure {name:?} has unknown kind {kind:?}"));
            }
            for field in ["spec", "evidence", "server_version"] {
                known_failure_field(entry, &name, field)?;
            }
            Ok((name, reason.to_owned()))
        })
        .collect()
}

fn known_failure_field<'a>(
    entry: &'a toml::Table,
    name: &str,
    field: &str,
) -> Result<&'a str, String> {
    entry
        .get(field)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("known failure {name:?} is missing {field:?}"))
}

async fn probe(
    server: &ServerInfo,
    wanted_caps: impl IntoIterator<Item = &str>,
) -> Result<Welcome, String> {
    let nick = papo_test_server::unique("papo-probe");
    let endpoint = Endpoint::plaintext(
        server.plaintext_addr().ip().to_string(),
        server.plaintext_addr().port(),
    )
    .unwrap();
    let wanted_caps = wanted_caps.into_iter().map(str::to_owned).collect();
    let mut connection = connect_client(
        server,
        endpoint,
        &nick,
        "papo conformance probe",
        Some(wanted_caps),
        None,
    )?;
    let welcome = wait_connected(&connection.client, &mut connection.events).await?;
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
        for feature in wanted_caps
            .iter()
            .filter_map(|name| unstable_feature_for_cap(name))
        {
            builder = builder.enable(feature);
        }
        builder = builder.request_caps(wanted_caps);
    }
    let bouncer = server
        .bouncer_login(nick)
        .map_err(|error| error.to_string())?;
    match (bouncer, sasl) {
        (Some(login), _) => {
            builder = builder
                .username(login.username())
                .password(login.password());
        }
        (None, Some(sasl)) => builder = builder.sasl(sasl),
        (None, None) => {}
    }
    let (client, events, driver) = builder.build();
    Ok(Connection {
        client,
        events,
        driver: tokio::spawn(driver),
    })
}

fn unstable_feature_for_cap(name: &str) -> Option<papo::Feature> {
    match name {
        "soju.im/bouncer-networks" | "soju.im/bouncer-networks-notify" => {
            Some(papo::Feature::Bouncer)
        }
        "draft/persistence" => Some(papo::Feature::Persistence),
        "draft/relaymsg" => Some(papo::Feature::Relay),
        "draft/webpush" | "soju.im/webpush" => Some(papo::Feature::WebPush),
        _ => None,
    }
}

async fn wait_connected(client: &Client, events: &mut Events) -> Result<Welcome, String> {
    timeout(TIMEOUT, async {
        loop {
            match events.next().await {
                Some(Event::Connected(connected)) => {
                    return Ok(Welcome {
                        caps: connected.caps,
                        isupport: client.state().isupport().clone(),
                    });
                }
                Some(Event::Disconnected(disconnected)) => {
                    return Err(disconnected.to_string());
                }
                Some(_) => {}
                None => return Err("driver stopped while connecting".into()),
            }
        }
    })
    .await
    .map_err(|_| "connect timed out".to_owned())?
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread tokio runtime")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        future::{self, Future},
        path::{Path, PathBuf},
        time::Duration,
    };

    use papo_core::Capabilities;
    use papo_proto::Isupport;

    use super::unstable_feature_for_cap;
    use super::{
        CapRequirement, Event, Requirements, Scenario, json_string, load_known_failures_from,
        runtime,
    };

    trait EventSource {
        fn next_event(&mut self) -> impl Future<Output = Option<Event>>;
    }

    impl EventSource for super::Events {
        fn next_event(&mut self) -> impl Future<Output = Option<Event>> {
            self.next()
        }
    }

    impl super::IsupportView for Isupport {
        fn advertised(&self, token: &str) -> papo::isupport::Advertised<'_> {
            self.get(token)
        }
    }

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

    async fn expect_event(
        events: &mut impl EventSource,
        mut matcher: impl FnMut(&Event) -> bool,
        limit: Duration,
    ) -> Result<Event, String> {
        tokio::time::timeout(limit, async {
            loop {
                match events.next_event().await {
                    Some(event) if matcher(&event) => return Ok(event),
                    Some(Event::Disconnected(disconnected)) => {
                        return Err(disconnected.to_string());
                    }
                    Some(_) => {}
                    None => return Err("driver stopped while waiting for an event".into()),
                }
            }
        })
        .await
        .map_err(|_| "timed out waiting for an event".to_owned())?
    }

    #[test]
    fn probe_capabilities_enable_their_unstable_features() {
        for (name, expected) in [
            ("soju.im/bouncer-networks", papo::Feature::Bouncer),
            ("soju.im/bouncer-networks-notify", papo::Feature::Bouncer),
            ("draft/persistence", papo::Feature::Persistence),
            ("draft/relaymsg", papo::Feature::Relay),
            ("draft/webpush", papo::Feature::WebPush),
            ("soju.im/webpush", papo::Feature::WebPush),
        ] {
            assert_eq!(unstable_feature_for_cap(name), Some(expected));
        }
        assert_eq!(unstable_feature_for_cap("chathistory"), None);
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
                Event::Warning(papo::event::Warning::Unsupported(papo::Unsupported::new(
                    papo::Feature::History,
                    papo::UnsupportedReason::NotOffered,
                ))),
                Event::Warning(papo::event::Warning::Unsupported(papo::Unsupported::new(
                    papo::Feature::Monitor,
                    papo::UnsupportedReason::NotOffered,
                ))),
            ]
            .into(),
        };
        let event = runtime()
            .block_on(expect_event(
                &mut events,
                |event| matches!(event, Event::Warning(papo::event::Warning::Unsupported(unsupported)) if unsupported.feature == papo::Feature::Monitor),
                Duration::from_secs(1),
            ))
            .unwrap();
        assert!(
            matches!(event, Event::Warning(papo::event::Warning::Unsupported(unsupported)) if unsupported.feature == papo::Feature::Monitor)
        );
    }

    #[test]
    fn expect_times_out_after_only_non_matching_events() {
        let mut events = ScriptedEvents {
            events: [Event::Warning(papo::event::Warning::Unsupported(
                papo::Unsupported::new(papo::Feature::Monitor, papo::UnsupportedReason::NotOffered),
            ))]
            .into(),
        };
        let error = runtime()
            .block_on(expect_event(
                &mut events,
                |_| false,
                Duration::from_millis(1),
            ))
            .err()
            .unwrap();
        assert_eq!(error, "timed out waiting for an event");
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
        let missing = load_known_failures_from(&directory.0, "ergo").unwrap();
        assert!(missing.is_empty());
        assert!(load_known_failures_from(&directory.0, "").is_err());
        assert!(load_known_failures_from(&directory.0, "../ergo").is_err());

        directory.write(
            "[connect]\nreason = 'server bug 417'\nkind = 'server-bug'\nspec = 'RFC 2812 section 4.1'\nevidence = '417'\nserver_version = '1.0'\n\
             [ping]\nreason = 'missing PONG'\nkind = 'not-implemented'\nspec = 'RFC 2812 section 3.7.2'\nevidence = 'PING received, no PONG'\nserver_version = '1.0'\n",
        );
        let parsed = load_known_failures_from(&directory.0, "ergo").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["connect"], "server bug 417");
        assert_eq!(parsed["ping"], "missing PONG");

        for (input, expected) in [
            ("[connect]\nnote = 'missing'\n", "missing \"reason\""),
            ("[connect]\nreason = ''\n", "missing \"reason\""),
            (
                "[connect]\nreason = 'broken'\nkind = 'implementation-bug'\nspec = 'RFC 2812'\nevidence = '417'\nserver_version = '1.0'\n",
                "unknown kind",
            ),
            (
                "[connect]\nreason = 'broken'\nkind = 'server-bug'\nspec = ''\n",
                "missing \"spec\"",
            ),
            (
                "[connect]\nreason = 'broken'\nkind = 'server-bug'\nspec = 'RFC 2812'\nevidence = '417'\n",
                "missing \"server_version\"",
            ),
            (
                "[connect\nreason = 'broken'\n",
                "invalid known-failures TOML",
            ),
        ] {
            directory.write(input);
            let error = load_known_failures_from(&directory.0, "ergo").unwrap_err();
            assert!(error.contains(expected), "{error:?}");
        }
    }

    #[test]
    fn json_strings_escape_control_characters() {
        assert_eq!(
            json_string("quote \" and\ncontrol\u{1f}"),
            "\"quote \\\" and\\ncontrol\\u001f\""
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn scenario_panics_become_test_failures() {
        let result = super::catch_scenario_panic(|| panic!("identity was not synchronized"));
        let Err(failure) = result else {
            panic!("scenario panic unexpectedly passed")
        };
        assert_eq!(
            failure.to_string(),
            "scenario panicked: identity was not synchronized"
        );
    }

    #[test]
    fn scenario_unique_names_keep_the_prefix_and_differ() {
        let scenario = Scenario {
            server: (),
            name: "connect",
            fixture: None,
            json: false,
        };
        let first = papo_test_server::unique(scenario.name);
        let second = papo_test_server::unique(scenario.name);
        assert!(first.starts_with("connect-"), "{first:?}");
        assert!(second.starts_with("connect-"), "{second:?}");
        assert_ne!(first, second);
    }
}
