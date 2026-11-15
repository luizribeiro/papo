//! Reads the requests and their results out of the public API snapshots.

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
