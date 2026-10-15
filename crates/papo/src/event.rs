use papo_core::{Capabilities, StateLimit, Welcome, sts::StsAction};
use papo_proto::Message;

/// An event emitted by an IRC client.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// Connection registration completed.
    Registered(Welcome),
    /// A replacement connection completed registration.
    Reconnected(Welcome),
    /// A parsed message not consumed by protocol behavior.
    Message(Message),
    /// A line from the server could not be decoded or parsed.
    Malformed {
        /// The original bytes, excluding the line ending.
        line: Vec<u8>,
    },
    /// Configured authentication failed and registration is continuing.
    AuthenticationFailed {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
    },
    /// The capabilities enabled on an established connection changed.
    CapsChanged {
        /// Newly enabled capabilities and updated advertised values.
        added: Capabilities,
        /// Capabilities removed by the server.
        removed: Capabilities,
    },
    /// A plaintext connection must be replaced with TLS on `port`.
    StsUpgrade {
        /// The advertised TLS port.
        port: u16,
    },
    /// An STS persistence action for the client to store.
    StsPolicy(StsAction),
    /// A user joined a channel shared with the client.
    Joined {
        /// The joined channel.
        channel: String,
        /// The joining user's nickname.
        nick: String,
    },
    /// Initial channel membership synchronization completed.
    ChannelSynced {
        /// The synchronized channel.
        channel: String,
    },
    /// Server input exceeded a configured state-retention bound.
    StateLimitReached {
        /// The state collection that rejected another entry.
        limit: StateLimit,
    },
    /// The connection ended.
    Disconnected {
        /// A human-readable explanation suitable for diagnostics.
        reason: String,
        /// Whether the client will try to connect again.
        will_retry: bool,
    },
}

pub(crate) enum CoreEvent {
    Forward(Event),
    Close(String),
    Unknown,
}

pub(crate) fn from_core(event: papo_core::Event) -> CoreEvent {
    CoreEvent::Forward(match event {
        papo_core::Event::Registered(welcome) => Event::Registered(welcome),
        papo_core::Event::Message(message) => Event::Message(message),
        papo_core::Event::Malformed { line } => Event::Malformed { line },
        papo_core::Event::Close { reason } => return CoreEvent::Close(reason),
        papo_core::Event::AuthenticationFailed { reason } => Event::AuthenticationFailed { reason },
        papo_core::Event::CapsChanged { added, removed } => Event::CapsChanged { added, removed },
        papo_core::Event::StsUpgrade { port } => Event::StsUpgrade { port },
        papo_core::Event::StsPolicy(action) => Event::StsPolicy(action),
        papo_core::Event::Joined { channel, nick } => Event::Joined { channel, nick },
        papo_core::Event::ChannelSynced { channel } => Event::ChannelSynced { channel },
        papo_core::Event::StateLimitReached { limit } => Event::StateLimitReached { limit },
        _ => return CoreEvent::Unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::{CoreEvent, Event, from_core};
    use papo_core::StateLimit;

    fn forwarded(event: papo_core::Event) -> Event {
        let CoreEvent::Forward(event) = from_core(event) else {
            panic!("event was not forwarded")
        };
        event
    }

    #[test]
    fn maps_joined() {
        assert_eq!(
            forwarded(papo_core::Event::Joined {
                channel: "#papo".into(),
                nick: "emily".into()
            }),
            Event::Joined {
                channel: "#papo".into(),
                nick: "emily".into()
            }
        );
    }

    #[test]
    fn maps_channel_synced() {
        assert_eq!(
            forwarded(papo_core::Event::ChannelSynced {
                channel: "#papo".into()
            }),
            Event::ChannelSynced {
                channel: "#papo".into()
            }
        );
    }

    #[test]
    fn maps_state_limit_reached() {
        assert_eq!(
            forwarded(papo_core::Event::StateLimitReached {
                limit: StateLimit::Members
            }),
            Event::StateLimitReached {
                limit: StateLimit::Members
            }
        );
    }
}
