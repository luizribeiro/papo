//! Checks that every request resolving to `()` names why the protocol gives it no confirmation,
//! and that every other request result is a plain value or an `*Outcome`.

use std::collections::BTreeSet;

const SNAPSHOTS: [&str; 4] = [
    include_str!("../public-api.txt"),
    include_str!("../public-api-unstable.txt"),
    include_str!("../../papo-core/public-api.txt"),
    include_str!("../../papo-core/public-api-unstable.txt"),
];

#[derive(Clone, Copy, Debug)]
enum Reason {
    /// No line reaches the server; the driver acts locally.
    LocalOnly,
    /// The server answers with a numeric whose text is fixed.
    FixedTextNumeric,
    /// The server's answer repeats only what was sent.
    EchoesInputOnly,
    /// The line is written without correlation; `request_raw` is the correlated form.
    UnlabelledByDesign,
    /// The connection closing is the confirmation.
    CloseIsConfirmation,
    /// The sans-IO session only queues the line; its result arrives as a typed event.
    QueuedEventCarriesResult,
    /// The notification is never stored or referenced, so its echo carries nothing to keep.
    Ephemeral,
}

const UNIT_REQUESTS: &[(&str, Reason)] = &[
    ("papo::Bouncer::remove", Reason::EchoesInputOnly),
    ("papo::ChangeNetwork", Reason::EchoesInputOnly),
    ("papo::Client::clear_away", Reason::FixedTextNumeric),
    ("papo::Client::invite", Reason::EchoesInputOnly),
    ("papo::Client::oper", Reason::FixedTextNumeric),
    ("papo::Client::pause_reconnect", Reason::LocalOnly),
    ("papo::Client::reconnect", Reason::LocalOnly),
    ("papo::Client::resume_reconnect", Reason::LocalOnly),
    ("papo::Client::send_raw", Reason::UnlabelledByDesign),
    ("papo::Client::set_away", Reason::FixedTextNumeric),
    ("papo::Client::set_typing", Reason::Ephemeral),
    ("papo::Metadata::remove", Reason::EchoesInputOnly),
    ("papo::Metadata::remove_on", Reason::EchoesInputOnly),
    ("papo::Metadata::unsubscribe", Reason::EchoesInputOnly),
    ("papo::QuitRequest", Reason::CloseIsConfirmation),
    ("papo::WebPush::unregister", Reason::EchoesInputOnly),
    (
        "papo_core::Session::create_account",
        Reason::QueuedEventCarriesResult,
    ),
    (
        "papo_core::Session::persistence_get",
        Reason::QueuedEventCarriesResult,
    ),
    (
        "papo_core::Session::persistence_set",
        Reason::QueuedEventCarriesResult,
    ),
    ("papo_core::Session::send", Reason::QueuedEventCarriesResult),
    (
        "papo_core::Session::verify",
        Reason::QueuedEventCarriesResult,
    ),
];

#[derive(Clone, Copy, Debug)]
enum Plain {
    Handle,
    List,
    Page,
    Value,
}

const PLAIN_RESULTS: &[(&str, Plain)] = &[
    ("alloc::vec::Vec<papo::BouncerNetwork>", Plain::List),
    ("alloc::vec::Vec<papo::Line>", Plain::List),
    ("alloc::vec::Vec<papo::ListEntry>", Plain::List),
    ("alloc::vec::Vec<papo::MetadataKey>", Plain::List),
    ("alloc::vec::Vec<papo::ModeListEntry>", Plain::List),
    ("alloc::vec::Vec<papo::RecentConversation>", Plain::List),
    ("alloc::vec::Vec<papo::WhoEntry>", Plain::List),
    ("core::option::Option<papo::Event>", Plain::Value),
    ("core::option::Option<papo::Whois>", Plain::Value),
    ("core::time::Duration", Plain::Value),
    ("papo::ChannelRef", Plain::Handle),
    ("papo::event::HistoryPage", Plain::Page),
    ("papo::MetadataValue", Plain::Value),
    ("papo::NetworkId", Plain::Value),
    ("papo::event::PersistenceStatus", Plain::Value),
    ("papo::ReadMarker", Plain::Value),
    (
        "std::collections::hash::map::HashMap<papo::MetadataKey, core::option::Option<papo::MetadataValue>>",
        Plain::List,
    ),
    (
        "std::collections::hash::map::HashMap<papo::MetadataKey, papo::MetadataValue>",
        Plain::List,
    ),
];

/// The item a snapshot line declares, with the request result it resolves to.
fn request(line: &str) -> Option<(&str, &str)> {
    let line = line.split(" where ").next()?;
    if let Some(field) = line.strip_prefix("pub ")
        && let Some((path, ty)) = field.split_once(": ")
        && !path.contains(' ')
    {
        return Some((
            path,
            ty.strip_prefix("core::result::Result<")?
                .strip_suffix(", papo::Error>")?,
        ));
    }
    if let Some(alias) = line.strip_prefix("pub type ") {
        let (path, ty) = alias.split_once("::IntoFuture = papo::RequestFuture<")?;
        return Some((path, ty.strip_suffix('>')?));
    }
    let function = line
        .split_once("pub fn ")
        .or_else(|| line.split_once("pub async fn "))?
        .1;
    let path = function.split(['(', '<']).next()?;
    let (_, returned) = function.rsplit_once(") -> ")?;
    if let Some(ty) = returned.strip_prefix("papo::RequestFuture<") {
        return Some((path, ty.strip_suffix('>')?));
    }
    path.starts_with("papo_core::Session::")
        .then(|| {
            returned
                .strip_prefix("core::result::Result<")?
                .strip_suffix(", papo_core::RequestError>")
        })
        .flatten()
        .map(|ty| (path, ty))
}

fn requests() -> BTreeSet<(&'static str, &'static str)> {
    SNAPSHOTS
        .iter()
        .flat_map(|snapshot| snapshot.lines())
        .filter_map(request)
        .collect()
}

fn assert_listed<T: std::fmt::Debug>(found: &BTreeSet<&str>, listed: &[(&str, T)], unlisted: &str) {
    let listed: BTreeSet<_> = listed.iter().map(|(item, _)| *item).collect();
    let missing: Vec<_> = found.difference(&listed).collect();
    assert!(missing.is_empty(), "{unlisted}: {missing:?}");
    let stale: Vec<_> = listed.difference(found).collect();
    assert!(stale.is_empty(), "no longer in the public API: {stale:?}");
}

#[test]
fn every_unit_request_names_why_it_has_no_confirmation() {
    let unit = requests()
        .into_iter()
        .filter(|(_, ty)| *ty == "()")
        .map(|(path, _)| path)
        .collect();
    assert_listed(&unit, UNIT_REQUESTS, "classify why it has no confirmation");
}

#[test]
fn every_other_request_result_is_a_plain_value_or_an_outcome() {
    let results = requests()
        .into_iter()
        .filter(|(path, _)| path.starts_with("papo::"))
        .map(|(_, ty)| ty)
        .filter(|ty| *ty != "()" && !ty.ends_with("Outcome") && !ty.contains("Outcome<"))
        .collect();
    assert_listed(
        &results,
        PLAIN_RESULTS,
        "name the result *Outcome or list it as a plain value",
    );
}

#[test]
fn request_reads_every_shape_of_snapshot_line() {
    let cases = [
        (
            "pub fn papo::Client::quit(&self, impl core::convert::Into<alloc::string::String>) -> papo::RequestFuture<()>",
            Some(("papo::Client::quit", "()")),
        ),
        (
            "pub fn papo::Metadata::unsubscribe<I, S>(&self, I) -> papo::RequestFuture<()> where I: core::iter::traits::collect::IntoIterator<Item = S>, S: core::convert::Into<papo::MetadataKey>",
            Some(("papo::Metadata::unsubscribe", "()")),
        ),
        (
            "pub type papo::ChangeNetwork::IntoFuture = papo::RequestFuture<()>",
            Some(("papo::ChangeNetwork", "()")),
        ),
        (
            "pub papo::KickbanOutcome::kick: core::result::Result<papo::KickOutcome, papo::Error>",
            Some(("papo::KickbanOutcome::kick", "papo::KickOutcome")),
        ),
        (
            "pub fn papo_core::Session::send(&mut self, std::time::Instant, papo::Line) -> core::result::Result<(), papo_core::RequestError>",
            Some(("papo_core::Session::send", "()")),
        ),
        (
            "pub fn papo_core::Session::request(&mut self, std::time::Instant, papo::Line) -> core::result::Result<papo_core::RequestId, papo_core::RequestError>",
            Some(("papo_core::Session::request", "papo_core::RequestId")),
        ),
        (
            "pub fn papo_core::Response::error(&self) -> core::result::Result<(), papo_core::RequestError>",
            None,
        ),
        (
            "pub type papo::Driver::Output = core::result::Result<(), papo::Error>",
            None,
        ),
        (
            "pub fn papo::Client::state(&self) -> alloc::sync::Arc<papo::State>",
            None,
        ),
    ];
    for (line, expected) in cases {
        assert_eq!(request(line), expected, "{line}");
    }
}
