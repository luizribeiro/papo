#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PeerScope {
    pub(super) server: String,
    pub(super) bouncer: Option<BouncerScope>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct BouncerScope {
    pub(super) login: String,
    pub(super) network: String,
    pub(super) upstream: String,
}
