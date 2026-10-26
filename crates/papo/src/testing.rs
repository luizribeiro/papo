//! Internal construction hooks for the unpublished `papo-testing` crate.

use std::{sync::Arc, time::SystemTime};

use papo_core::{Config, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    AccountName, Authorship, Capabilities, Casemapping, ChannelName, Mask, Message, MessageKind,
    Nick, Source, State, Tags, UserSource,
    event::{AwayChanged, Connected, Joined},
};

/// Constructs an origin-free nickname for test fixtures.
#[must_use]
pub fn nickname(casemapping: Casemapping, text: impl AsRef<str>) -> Nick {
    casemapping.nick(text)
}

/// Constructs an origin-free channel name for test fixtures.
#[must_use]
pub fn channel_name(casemapping: Casemapping, text: impl AsRef<str>) -> ChannelName {
    casemapping.channel(text)
}

/// Constructs an origin-free account name for test fixtures.
#[must_use]
pub fn account_name(casemapping: Casemapping, text: impl AsRef<str>) -> AccountName {
    casemapping.account(text)
}

/// Constructs an origin-free mask for test fixtures.
#[must_use]
pub fn mask(casemapping: Casemapping, text: impl AsRef<str>) -> Mask {
    casemapping.mask(text)
}

/// Builds a client whose connection attempts use in-memory byte streams.
pub fn wire_client(
    builder: crate::Builder,
) -> (
    crate::Client,
    crate::Events,
    crate::Driver,
    tokio::sync::mpsc::UnboundedReceiver<(tokio::io::DuplexStream, String)>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let connector: crate::driver::Connector = Arc::new(move |plan, _| {
        let (client, server) = tokio::io::duplex(8192);
        let sent = sender.send((server, plan.address.to_string()));
        Box::pin(async move {
            sent.map_err(|_| crate::Error::connection_lost())?;
            Ok(Box::new(client) as Box<dyn crate::rt::Io>)
        })
    });
    let (client, events, driver) = builder.build_with_connector(connector);
    (client, events, driver, receiver)
}

/// Writes exact server bytes to an in-memory client connection.
///
/// # Panics
///
/// Panics if the in-memory pipe cannot accept all bytes.
pub async fn server_line(server: &mut tokio::io::DuplexStream, line: &[u8]) {
    server.write_all(line).await.unwrap();
}

/// Reads and checks exact client bytes from an in-memory connection.
///
/// # Panics
///
/// Panics if the client closes early or sends bytes other than `expected`.
pub async fn client_line(server: &mut tokio::io::DuplexStream, expected: &[u8]) {
    let mut actual = vec![0; expected.len()];
    server.read_exact(&mut actual).await.unwrap();
    assert_eq!(actual, expected);
}

/// Constructs a user event source.
#[must_use]
pub fn user_source(
    nick: impl Into<Nick>,
    username: Option<String>,
    host: Option<String>,
) -> UserSource {
    let nick = nick.into();
    let source = Source::from_proto(
        papo_proto::Source::__user(&nick, username.as_deref(), host.as_deref()),
        nick.casemapping(),
    );
    drop((username, host));
    let Source::User(source) = source else {
        unreachable!("a user prefix produces a user source")
    };
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
    actor: Option<UserSource>,
    target: impl Into<String>,
    text: impl Into<String>,
    private: bool,
) -> Message {
    let text = text.into();
    let target = target.into();
    let conversation = if private {
        let other = actor.as_ref().map_or(target.as_str(), |actor| actor.nick());
        crate::Conversation::Private(crate::Casemapping::default().nick(other))
    } else {
        crate::Conversation::Channel(crate::Casemapping::default().channel(&target))
    };
    Message {
        actor: actor.unwrap_or_else(|| {
            user_source(crate::Casemapping::default().nick("unknown"), None, None)
        }),
        authorship: Authorship::Other,
        conversation,
        status: None,
        raw_text: text.clone(),
        text,
        kind: MessageKind::Text,
        msgid: None,
        server_time: None,
        received_at: SystemTime::UNIX_EPOCH,
        in_reply_to: None,
        channel_context: None,
        #[cfg(feature = "unstable")]
        relayed_by: None,
        tags: Tags::default(),
        reply_token: None,
    }
}

/// Sets the account fact on a test message actor.
pub fn set_message_account(message: &mut Message, account: crate::Account) {
    message.actor.set_account(account);
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
        server_time: None,
        received_at: SystemTime::UNIX_EPOCH,
        realname: None,
        channel: channel.into(),
        actor: user_source(nick, None, None),
        authorship: if own {
            Authorship::Own
        } else {
            Authorship::Other
        },
        netjoin: None,
    }
}

/// Constructs an away-state event payload.
#[must_use]
pub fn away_changed(nick: impl Into<Nick>, away: crate::Away) -> AwayChanged {
    AwayChanged {
        actor: user_source(nick, None, None),
        new: away,
        authorship: Authorship::Other,
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

    #[tokio::test]
    async fn wire_client_connects_through_memory() {
        let builder = crate::Client::builder("irc://offline.test").nick("papo-wire");
        let (_client, _events, driver, mut connections) = super::wire_client(builder);
        let task = tokio::spawn(driver);
        let (mut server, endpoint) = connections.recv().await.unwrap();
        assert_eq!(endpoint, "irc://offline.test");
        let expected = b"CAP LS 302\r\nNICK papo-wire\r\nUSER papo-wire 0 * papo-wire\r\n";
        super::client_line(&mut server, expected).await;
        task.abort();
    }

    #[test]
    fn constructs_payloads_and_state_without_network_io() {
        let source = user_source(
            crate::Casemapping::default().nick("alice"),
            Some("a".into()),
            Some("cloak.example".into()),
        );
        let message = message(Some(source), "bob", "hello from alice", true);
        assert_eq!(message.return_address(), "alice");
        assert_eq!(message.raw_text, "hello from alice");

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
