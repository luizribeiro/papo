use papo_core::{NetworkConfig, NetworkId, PersistenceSetting, RequestError, RequestId, Session};

use super::{CoreRequest, now};

pub(crate) enum UnstableRequest {
    PersistenceGet,
    PersistenceSet(PersistenceSetting),
    BouncerNetworks,
    BouncerAdd(NetworkConfig),
    BouncerChange(NetworkId, NetworkConfig),
    BouncerRemove(NetworkId),
}

impl UnstableRequest {
    pub(super) fn start(self, session: &mut Session) -> Result<RequestId, RequestError> {
        let before = session.next_request_id();
        match self {
            Self::PersistenceGet => session.persistence_get(now())?,
            Self::PersistenceSet(setting) => session.persistence_set(now(), setting)?,
            Self::BouncerNetworks => return session.bouncer_networks(now()),
            Self::BouncerAdd(config) => return session.bouncer_add(now(), &config),
            Self::BouncerChange(id, config) => {
                return session.bouncer_change(now(), &id, &config);
            }
            Self::BouncerRemove(id) => return session.bouncer_remove(now(), &id),
        }
        Ok(before)
    }
}

impl From<UnstableRequest> for CoreRequest {
    fn from(request: UnstableRequest) -> Self {
        Self::Unstable(request)
    }
}
