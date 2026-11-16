#[derive(Debug)]
pub(crate) struct PeerState;

impl PeerState {
    pub(crate) const fn from_session(_: &papo_core::Session) -> Self {
        Self
    }

    #[expect(
        clippy::unused_self,
        reason = "shares its signature with the unstable peer state, which reads itself"
    )]
    pub(crate) fn scope(
        &self,
        _: &papo_core::State,
        _: &str,
        _: &str,
    ) -> Option<crate::peer_key::PeerScope> {
        None
    }
}
