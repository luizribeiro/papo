//! Internal construction hooks for the separately published testing crate.

use std::time::SystemTime;

use papo_core::{Config, Session};

use crate::{
    Capabilities, ChannelName, Message, MessageKind, Nick, Origin, Source, State, Tags, Target,
    event::{AwayChanged, Joined, Registered},
};

/// Constructs a user event source.
#[must_use]
pub fn user_source(nick: impl Into<Nick>, user: Option<String>, host: Option<String>) -> Source {
    Source::User {
        nick: nick.into(),
        user,
        host,
    }
}

/// Constructs owned message tags in wire order.
#[must_use]
pub fn tags(values: Vec<(String, Option<String>)>) -> Tags {
    Tags::from_pairs(values)
}

/// Constructs the minimal message payload extended by `papo-testing`.
#[must_use]
pub fn message(
    sender: Option<Source>,
    target: impl Into<Target>,
    text: impl Into<String>,
    private: bool,
) -> Message {
    let text = text.into();
    Message {
        sender,
        target: target.into(),
        status: None,
        raw_text: text.clone(),
        text,
        kind: MessageKind::Privmsg,
        msgid: None,
        server_time: None,
        received_at: SystemTime::UNIX_EPOCH,
        account: None,
        reply_to: None,
        channel_context: None,
        #[cfg(feature = "unstable")]
        relayed_by: None,
        is_bot: false,
        is_oper: false,
        oper: None,
        origin: Origin::Live,
        tags: Tags::default(),
        private,
    }
}

/// Constructs a registration event payload.
#[must_use]
pub fn registered(nick: impl Into<Nick>) -> Registered {
    Registered {
        nick: nick.into(),
        text: String::new(),
        caps: Capabilities::default(),
        reconnected: false,
    }
}

/// Constructs a channel-join event payload.
#[must_use]
pub fn joined(channel: impl Into<ChannelName>, nick: impl Into<Nick>, is_self: bool) -> Joined {
    Joined {
        channel: channel.into(),
        nick: nick.into(),
        is_self,
    }
}

/// Constructs an away-state event payload.
#[must_use]
pub fn away_changed(nick: impl Into<Nick>, message: Option<String>) -> AwayChanged {
    AwayChanged {
        nick: nick.into(),
        message,
    }
}

/// Constructs a minimal immutable client-state snapshot.
#[must_use]
pub fn state(nick: impl Into<String>, connected: bool) -> State {
    let session = Session::new(Config::default().nick(nick));
    State::from_session(&session, connected)
}

#[cfg(all(test, not(browser)))]
mod tests {
    use super::{joined, message, state, user_source};

    #[test]
    fn constructs_payloads_and_state_without_network_io() {
        let source = user_source("alice", Some("a".into()), Some("cloak.example".into()));
        let message = message(Some(source), "bob", "hello from alice", true);
        assert_eq!(message.reply_target(), "alice");
        assert_eq!(message.raw_text(), "hello from alice");

        let joined = joined("#papo", "bob", true);
        assert_eq!(joined.channel.as_ref(), "#papo");
        assert!(joined.is_self);

        let snapshot = state("bob", true);
        assert_eq!(snapshot.nick().as_ref(), "bob");
        assert!(snapshot.is_connected());
    }
}
