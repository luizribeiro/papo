use papo_core::{Capabilities, Session};
use papo_proto::Isupport;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct State {
    pub(crate) core: papo_core::State,
    pub(crate) caps: Capabilities,
    pub(crate) isupport: Isupport,
    pub(crate) account: Option<String>,
    pub(crate) connected: bool,
    generation: u64,
}

impl State {
    pub(crate) fn from_session(session: &Session, connected: bool) -> Self {
        Self {
            core: session.state().clone(),
            caps: session.caps().clone(),
            isupport: session.isupport().clone(),
            account: session.account().map(str::to_owned),
            connected,
            generation: session.state_generation(),
        }
    }

    pub(crate) const fn matches(&self, session: &Session, connected: bool) -> bool {
        self.generation == session.state_generation() && self.connected == connected
    }
}
