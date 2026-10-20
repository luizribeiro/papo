//! Internal construction hooks for the separately published testing crate.

use std::time::SystemTime;

use papo_core::{Config, Session};

use crate::{
    Authorship, Capabilities, ChannelName, Conversation, Message, MessageKind, Nick, Origin,
    Source, State, Tags,
    event::{AwayChanged, Connected, Joined},
};

/// Constructs a user event source.
#[must_use]
pub fn user_source(
    nick: impl Into<Nick>,
    username: Option<String>,
    host: Option<String>,
) -> Source {
    let nick = nick.into();
    let source = Source::from_proto(
        papo_proto::Source::User {
            nick: &nick,
            username: username.as_deref(),
            host: host.as_deref(),
        },
        nick.casemapping(),
    );
    drop((username, host));
    source
}

/// Constructs owned message tags in wire order.
#[must_use]
pub fn tags(values: Vec<(String, Option<String>)>) -> Tags {
    Tags::from_pairs(values)
}

/// Constructs the minimal message payload extended by `papo-testing`.
#[must_use]
pub fn message(
    actor: Option<Source>,
    target: impl Into<String>,
    text: impl Into<String>,
    private: bool,
) -> Message {
    let text = text.into();
    let target = target.into();
    let conversation = if private {
        let other: &str = match actor.as_ref().and_then(Source::nick) {
            Some(nick) => AsRef::<str>::as_ref(nick),
            None => &target,
        };
        Conversation::Private(crate::Casemapping::default().nick(other))
    } else {
        Conversation::Channel(crate::Casemapping::default().channel(&target))
    };
    Message {
        actor: actor.unwrap_or_else(|| {
            Source::from_proto(
                papo_proto::Source::Server("irc.example"),
                crate::Casemapping::default(),
            )
        }),
        authorship: Authorship::Other,
        conversation,
        status: None,
        raw_text: text.clone(),
        text,
        kind: MessageKind::Privmsg,
        msgid: None,
        time: SystemTime::UNIX_EPOCH,
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
    }
}

/// Constructs a connected event payload.
#[must_use]
pub fn connected(nick: impl Into<Nick>) -> Connected {
    Connected {
        nick: nick.into(),
        welcome: String::new(),
        caps: Capabilities::default(),
        reconnected: false,
    }
}

/// Constructs a channel-join event payload.
#[must_use]
pub fn joined(channel: impl Into<ChannelName>, nick: impl Into<Nick>, own: bool) -> Joined {
    let nick = nick.into();
    Joined {
        msgid: None,
        time: SystemTime::UNIX_EPOCH,
        account: crate::Account::Unknown,
        realname: None,
        channel: channel.into(),
        actor: user_source(nick.clone(), None, None),
        nick,
        authorship: if own {
            Authorship::Own
        } else {
            Authorship::Other
        },
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
    use super::{connected, joined, message, state, user_source};

    #[test]
    fn constructs_payloads_and_state_without_network_io() {
        let source = user_source(
            crate::Casemapping::default().nick("alice"),
            Some("a".into()),
            Some("cloak.example".into()),
        );
        let message = message(Some(source), "bob", "hello from alice", true);
        assert_eq!(message.return_address(), "alice");
        assert_eq!(message.raw_text(), "hello from alice");

        let joined = joined(
            crate::Casemapping::default().channel("#papo"),
            crate::Casemapping::default().nick("bob"),
            true,
        );
        assert_eq!(joined.channel.as_ref(), "#papo");
        assert_eq!(joined.authorship, crate::Authorship::Own);

        let connected = connected(crate::Casemapping::default().nick("bob"));
        assert_eq!(connected.nick, "bob");
        assert!(connected.welcome.is_empty());
        assert!(!connected.reconnected);

        let snapshot = state("bob", true);
        assert_eq!(snapshot.nick().as_ref(), "bob");
        assert!(snapshot.is_connected());
    }
}
