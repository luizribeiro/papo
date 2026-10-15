//! Cross-server live conformance tests for papo.

#![cfg(any(
    not(target_family = "wasm"),
    all(target_os = "wasi", not(target_env = "p1"))
))]

use std::{future::Future, pin::Pin, process, time::Duration};

use libtest_mimic::{Arguments, Completion, Failed, Trial};
use papo::{Client, Driver, Event, Events, OsEntropy, transport};
use papo_core::{Config, Session, Welcome};
use papo_test_server::{ServerInfo, from_env};
use tokio::{runtime::Builder, task::JoinHandle, time::timeout};

const TIMEOUT: Duration = Duration::from_secs(10);

type ScenarioResult = Result<Completion, Failed>;
type ScenarioFuture = Pin<Box<dyn Future<Output = ScenarioResult>>>;

#[derive(Clone, Copy)]
struct ScenarioSpec {
    name: &'static str,
    doc: &'static str,
    run: fn(Scenario) -> ScenarioFuture,
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
    let probe = runtime()
        .block_on(probe(&server))
        .unwrap_or_else(|error| startup_error(&error));
    let trials = scenario_specs()
        .iter()
        .copied()
        .map(|spec| trial(spec, server.clone(), probe.clone()))
        .collect();
    libtest_mimic::run(&arguments, trials).exit();
}

fn scenario_specs() -> &'static [ScenarioSpec] {
    &[]
}

fn trial(spec: ScenarioSpec, server: ServerInfo, _probe: Welcome) -> Trial {
    Trial::ignorable_test(spec.name, move || {
        runtime().block_on((spec.run)(Scenario { _server: server }))
    })
    .with_kind(spec.doc)
}

async fn probe(server: &ServerInfo) -> Result<Welcome, String> {
    let nick = papo_test_server::unique("papo-probe");
    let config = Config::default()
        .nick(&nick)
        .username("papo")
        .realname("papo conformance probe")
        .wanted_caps(["sasl", "echo-message"]);
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
