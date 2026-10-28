//! Internal construction hooks for the unpublished `papo-testing` crate.

use std::{sync::Arc, time::SystemTime};

use papo_core::{Config, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    AccountName, Authorship, Capabilities, Casemapping, ChannelName, Mask, Message, MessageKind,
    Nick, Source, State, Tags, UserSource,
    event::{AwayChanged, Connected, Joined},
};

/// Exercises a complete core session and observes public events mapped from arbitrary server bytes.
#[doc(hidden)]
pub fn exercise_session_mapping(input: &[u8], mut observe: impl FnMut(crate::Event)) {
    let now = web_time::Instant::now();
    let mut session = Session::new(Config::default());
    session.handle_connected(now);
    for line in [
        b"CAP * LS :extended-monitor".as_slice(),
        b"CAP * ACK :extended-monitor".as_slice(),
        b":irc.example 001 papo :Welcome".as_slice(),
        b":irc.example 005 papo MONITOR=100 :are supported".as_slice(),
    ] {
        session.handle_line(now, line);
    }
    while session.poll_transmit().is_some() {}
    while session.poll_event().is_some() {}

    session.handle_bytes(now, input);
    while let Some(event) = session.poll_event() {
        let mapped = crate::event_impl::from_core(
            event,
            session.isupport().casemapping(),
            crate::Origin::NONE,
            session.caps().contains_key(&crate::cap::ACCOUNT_TAG),
            false,
            SystemTime::UNIX_EPOCH,
        );
        match mapped {
            crate::event_impl::CoreEvent::Forward(event) => observe(event),
            crate::event_impl::CoreEvent::Line(line) => {
                observe(crate::event_impl::incoming_from_core(
                    line,
                    crate::Origin::NONE,
                    session.isupport(),
                    false,
                    false,
                    SystemTime::UNIX_EPOCH,
                ));
            }
            _ => {}
        }
    }
}

/// Exercises session framing while checking batch accounting after every input byte.
///
/// Returns the number of bytes whose resulting state was checked.
#[doc(hidden)]
#[must_use]
pub fn exercise_session_mapping_invariants(input: &[u8]) -> usize {
    let now = web_time::Instant::now();
    let mut session = Session::new(Config::default());
    for byte in input {
        session.handle_bytes(now, std::slice::from_ref(byte));
        assert!(papo_core::test_support::batch_accounting_is_sound(&session));
    }
    input.len()
}

/// Constructs an origin-free nickname for test fixtures.
#[must_use]
pub fn nickname(casemapping: Casemapping, text: impl Into<String>) -> Nick {
    casemapping.nick(text)
}

/// Constructs an origin-free channel name for test fixtures.
#[must_use]
pub fn channel_name(casemapping: Casemapping, text: impl Into<String>) -> ChannelName {
    casemapping.channel(text)
}

/// Constructs an origin-free account name for test fixtures.
#[must_use]
pub fn account_name(casemapping: Casemapping, text: impl Into<String>) -> AccountName {
    casemapping.account(text)
}

/// Constructs an origin-free mask for test fixtures.
#[must_use]
pub fn mask(casemapping: Casemapping, text: impl Into<String>) -> Mask {
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
    let return_address = conversation.to_string();
    let actor = actor
        .unwrap_or_else(|| user_source(crate::Casemapping::default().nick("unknown"), None, None));
    let receipt = crate::message_ref::Receipt::new(
        crate::Source::User(actor.clone()),
        Authorship::Other,
        &conversation,
        None,
        return_address,
    );
    Message {
        actor,
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
        receipt,
    }
}

/// Sets the account fact on a test message actor.
pub fn set_message_account(message: &mut Message, account: crate::Account) {
    message.actor.set_account(account.clone());
    message.receipt.update_account(account);
}

/// Sets the actor and matching private conversation on a test message.
pub fn set_message_actor(message: &mut Message, actor: crate::UserSource) {
    if matches!(message.conversation, crate::Conversation::Private(_)) {
        message.conversation = crate::Conversation::Private(actor.nick().clone());
    }
    message.actor = actor;
    message.receipt.update_actor(
        crate::Source::User(message.actor.clone()),
        &message.conversation,
    );
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
        server_time: None,
        received_at: SystemTime::UNIX_EPOCH,
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
    use super::{
        connected, exercise_session_mapping, exercise_session_mapping_invariants, joined, message,
        set_message_account, set_message_actor, state, user_source,
    };

    #[test]
    fn session_mapping_seed_produces_a_private_message() {
        let mut mapped = None;
        exercise_session_mapping(
            b":alice!~alice@users.example PRIVMSG papo :release ready\r\n",
            |event| mapped = Some(event),
        );

        let Some(crate::Event::Message(message)) = mapped else {
            panic!("expected the seed line to map to a message")
        };
        assert_eq!(message.actor.nick(), "alice");
        assert_eq!(message.text, "release ready");
        assert_eq!(message.return_address(), "alice");
    }

    #[test]
    fn session_mapping_invariant_checks_every_input_byte() {
        let input = b":ergo.test BATCH +history chathistory #papo\r\n\
                      @batch=history :alice!~alice@users.example BATCH +multiline draft/multiline #papo\r\n\
                      @batch=multiline :alice!~alice@users.example PRIVMSG #papo :review is ready\r\n\
                      @batch=history :alice!~alice@users.example BATCH -multiline\r\n";
        assert_eq!(exercise_session_mapping_invariants(input), input.len());
    }

    #[tokio::test]
    async fn wire_client_connects_through_memory() {
        let builder = crate::Client::builder("irc://offline.test", "papo-wire");
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
        let channel = message(Some(source.clone()), "#papo", "release ready", false);
        assert!(matches!(
            &channel.conversation,
            crate::Conversation::Channel(_)
        ));
        assert_eq!(channel.return_address(), "#papo");

        let private = message(Some(source), "papo", "hello from alice", true);
        assert!(matches!(
            &private.conversation,
            crate::Conversation::Private(_)
        ));
        assert_eq!(private.return_address(), "alice");
        assert_eq!(private.raw_text, "hello from alice");

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

    #[test]
    fn sets_message_actor_account() {
        let mapping = crate::Casemapping::default();
        let source = user_source(
            mapping.nick("alice"),
            Some("~alice".into()),
            Some("localhost".into()),
        );
        let mut message = message(Some(source), "papo", "release ready", true);
        set_message_actor(
            &mut message,
            user_source(
                mapping.nick("emily"),
                Some("~emily".into()),
                Some("cloak.example".into()),
            ),
        );

        set_message_account(
            &mut message,
            crate::Account::LoggedIn(mapping.account("emily")),
        );

        assert_eq!(message.return_address(), "emily");
        assert!(
            matches!(message.actor.account(), crate::Account::LoggedIn(account) if account == "emily")
        );
        let recipient = message
            .receipt
            .recipient()
            .expect("private recipient guard");
        assert_eq!(recipient.nick, "emily");
        assert!(
            matches!(recipient.account, crate::Account::LoggedIn(account) if account == "emily")
        );
    }
}
