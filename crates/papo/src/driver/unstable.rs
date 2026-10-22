use papo_core::{
    NetworkConfig, NetworkId, PersistenceSetting, RequestError, RequestId, Secret, Session,
};

use super::{CoreRequest, now};

pub(crate) enum UnstableRequest {
    PersistenceGet,
    PersistenceSet(PersistenceSetting),
    BouncerNetworks,
    BouncerAdd(NetworkConfig),
    BouncerChange(NetworkId, NetworkConfig),
    BouncerRemove(NetworkId),
    Relay {
        channel: String,
        nick: String,
        text: String,
    },
    WebPushRegister {
        endpoint: String,
        p256dh: String,
        auth: Secret,
    },
    WebPushUnregister(String),
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
            Self::Relay {
                channel,
                nick,
                text,
            } => return session.relay(now(), &channel, &nick, &text),
            Self::WebPushRegister {
                endpoint,
                p256dh,
                auth,
            } => {
                let auth = auth.expose().ok_or(RequestError::InvalidSecret)?;
                return session.webpush_register(now(), &endpoint, &p256dh, auth);
            }
            Self::WebPushUnregister(endpoint) => {
                return session.webpush_unregister(now(), &endpoint);
            }
        }
        Ok(before)
    }
}

impl From<UnstableRequest> for CoreRequest {
    fn from(request: UnstableRequest) -> Self {
        Self::Unstable(request)
    }
}
