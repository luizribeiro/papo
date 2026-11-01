use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

use tokio::sync::mpsc;

use crate::Nick;

/// A stable, guarded identity for direct messages to one IRC user.
///
/// The handle follows nickname changes and refuses a send with [`crate::Error::TargetGone`] after
/// the identity leaves or the nickname changes hands. When the user shares no channel with the
/// client, papo reserves one server-side `MONITOR` entry until the last clone is dropped. Handles
/// compare by the tracked IRC user on one connection, regardless of when account information was
/// learned and across nickname changes. Different users remain unequal even when logged into the
/// same account, and a nickname reused after departure identifies a new user. Handles are per
/// connection: after a reconnect, a new handle does not equal an old one.
/// [`crate::Client::send_raw`] is the explicit escape hatch for deliberately unguarded IRC lines.
#[derive(Clone)]
pub struct DirectRef(Arc<DirectInner>);

struct DirectInner {
    identity: papo_core::ReplyToken,
    peer_key: crate::peer_key::PeerKey,
    recipient: papo_core::RecipientGuard,
    state: Arc<RwLock<Arc<crate::State>>>,
    _lease: Option<MonitorLease>,
}

pub(crate) struct MonitorLease {
    recipient: papo_core::RecipientGuard,
    commands: mpsc::Sender<crate::driver::Command>,
}

impl Drop for MonitorLease {
    fn drop(&mut self) {
        _ = self
            .commands
            .try_send(crate::driver::Command::DirectRelease {
                recipient: self.recipient.clone(),
            });
    }
}

impl DirectRef {
    pub(crate) fn new(
        client: &crate::Client,
        recipient: papo_core::RecipientGuard,
        monitored: bool,
    ) -> Self {
        let identity = recipient.__token();
        let state = client.state();
        let scope = state.peer_scope();
        let peer_key = crate::peer_key::PeerKey::live(
            scope.as_ref(),
            recipient.__account(),
            state.peer_token(recipient.__nick()),
            client.origin(),
        );
        Self(Arc::new(DirectInner {
            _lease: monitored.then(|| MonitorLease {
                recipient: recipient.clone(),
                commands: client.commands.clone(),
            }),
            identity,
            peer_key,
            recipient,
            state: Arc::clone(&client.state),
        }))
    }

    pub(crate) fn recipient(&self) -> &papo_core::RecipientGuard {
        &self.0.recipient
    }

    /// Returns the identity pin captured for this direct conversation.
    ///
    /// Nicknames remain the conversation boundary even when two references have equal pins. See
    /// [`crate::PeerKey`] for the nickname-keyed thread and warning flow.
    #[must_use]
    pub fn peer_key(&self) -> &crate::PeerKey {
        &self.0.peer_key
    }

    /// Returns the recipient's current nickname.
    ///
    /// After the user is gone, this returns their last known nickname. Sends still fail with
    /// [`crate::Error::TargetGone`].
    #[must_use]
    pub fn nick(&self) -> Nick {
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.resolve_recipient(&self.0.recipient) {
            papo_core::RecipientStatus::Current(nick) => nick,
            papo_core::RecipientStatus::Gone => state
                .last_recipient_nick(&self.0.recipient)
                .unwrap_or_else(|| self.0.recipient.__nick().clone()),
            papo_core::RecipientStatus::Unknown => self.0.recipient.__nick().clone(),
        }
    }
}

impl std::fmt::Debug for DirectRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectRef")
            .field("nick", &self.nick())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for DirectRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.nick().fmt(formatter)
    }
}

impl PartialEq for DirectRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.identity == other.0.identity
    }
}

impl Eq for DirectRef {}

impl Hash for DirectRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.identity.hash(state);
    }
}

impl crate::target::sealed::Sealed for DirectRef {}

impl crate::AsNick for DirectRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        <Self as crate::Target>::__parts(self)
    }
}

impl crate::Target for DirectRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        let nick = self.nick();
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(nick.to_string()),
            origin: nick.origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(
                &self.0.recipient,
            )),
            channel: None,
        }
    }
}

pub(crate) fn lease(client: &crate::Client, recipient: papo_core::RecipientGuard) -> MonitorLease {
    MonitorLease {
        recipient,
        commands: client.commands.clone(),
    }
}

impl crate::Client {
    /// Captures the identity currently using `nick` for guarded direct messages.
    ///
    /// A user who shares no channel is added to `MONITOR`; one `WHO nick %na` establishes the
    /// identity before this future resolves. The monitor entry is removed when the last handle
    /// owning it is dropped.
    ///
    /// ```no_run
    /// # async fn send(client: &papo::Client) -> Result<(), papo::Error> {
    /// let alice = client.direct("alice").await?;
    /// client.privmsg(&alice, "release approved").await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::MonitorFull`] when the server's advertised `MONITOR` limit is already
    /// occupied. It also returns an unsupported, server, invalid-input, timeout, or disconnected
    /// error if the identity cannot be established without failing open.
    pub fn direct(&self, nick: impl crate::AsNick) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("direct: nick"), nick)?;
            let snapshot = client.state();
            if let Some(user) = snapshot.user(&nick)
                && (user.nick() == snapshot.nick() || user.channels().len() != 0)
            {
                let recipient = snapshot.capture_recipient(user.nick(), user.account().clone());
                return Ok(DirectRef::new(&client, recipient, false));
            }
            crate::direct_runtime::require_monitor_slot(&snapshot, &nick)?;
            drop(snapshot);
            let recipient = crate::direct_runtime::acquire(&client, nick.clone()).await?;
            let (recipient, _) = crate::direct_runtime::prove(&client, nick, recipient).await?;
            Ok(DirectRef::new(&client, recipient, true))
        })
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        hash::{DefaultHasher, Hash, Hasher},
        sync::Arc,
        time::Duration,
    };

    use crate::{Account, Casemapping};
    use papo_core::{Config, Event, ResponseMatcher, Session};
    use papo_proto::{Line, LineRef};

    fn direct_hash(direct: &super::DirectRef) -> u64 {
        let mut hasher = DefaultHasher::new();
        direct.hash(&mut hasher);
        hasher.finish()
    }

    fn response_with(lines: &[&str]) -> papo_core::Response {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default());
        let id = session
            .request_with_matcher(
                now,
                Line::new("WHO").with_params(["alice", "%na"]),
                ResponseMatcher::Who("alice".into()),
            )
            .unwrap();
        _ = session.poll_transmit();
        session.handle_line(now, b":ergo.test 315 papo alice :End of WHO list");
        let mut response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) => Some(response),
                _ => None,
            })
            .unwrap();
        assert_eq!(response.id, id);
        response.lines = lines
            .iter()
            .map(|line| LineRef::parse(line).unwrap().to_owned())
            .collect();
        response
    }

    async fn next_command(
        commands: &mut tokio::sync::mpsc::Receiver<crate::driver::Command>,
    ) -> crate::driver::Command {
        tokio::time::timeout(Duration::from_millis(50), commands.recv())
            .await
            .expect("driver did not receive a direct-message command")
            .expect("driver command channel closed")
    }

    #[test]
    fn direct_references_are_nickname_arguments() {
        fn assert_as_nick<T: crate::AsNick>() {}

        assert_as_nick::<super::DirectRef>();
    }

    #[test]
    fn dropping_a_monitor_lease_releases_its_recipient() {
        let (commands, mut receiver) = tokio::sync::mpsc::channel(1);
        let nick = Casemapping::default().nick("alice");
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick,
            Account::LoggedIn(Casemapping::default().account("alice-account")),
            papo_core::ReplyToken::__unknown(crate::Origin::NONE),
        );
        drop(super::MonitorLease {
            recipient: recipient.clone(),
            commands,
        });

        assert!(matches!(
            receiver.try_recv(),
            Ok(crate::driver::Command::DirectRelease { recipient: released })
                if released == recipient
        ));
    }

    #[test]
    fn direct_references_use_stable_captured_identity() {
        let mut session = Session::new(Config::default().nick("papo"));
        let prove = |session: &mut Session, nick: &str, account: Account| {
            let acquired = session.__direct_acquire(nick, None, None, Account::Unknown);
            session.__direct_prove(&acquired, account).unwrap()
        };
        let account = Account::LoggedIn(session.isupport().casemapping().account("alice-account"));
        let desktop = prove(&mut session, "alice", account.clone());
        let desktop_unverified = papo_core::RecipientGuard::__from_parts(
            desktop.__nick().clone(),
            Account::Unknown,
            desktop.__token(),
        );
        let phone = prove(&mut session, "alice-phone", account);
        let mallory_account =
            Account::LoggedIn(session.isupport().casemapping().account("mallory-account"));
        let mallory = prove(&mut session, "mallory", mallory_account);
        let bob = prove(&mut session, "bob", Account::LoggedOut);
        let carol = prove(&mut session, "carol", Account::LoggedOut);
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let desktop = super::DirectRef::new(&client, desktop, false);
        let desktop_unverified = super::DirectRef::new(&client, desktop_unverified, false);
        let phone = super::DirectRef::new(&client, phone, false);
        let mallory = super::DirectRef::new(&client, mallory, false);
        let bob = super::DirectRef::new(&client, bob, false);
        let carol = super::DirectRef::new(&client, carol, false);

        assert_ne!(desktop, phone);
        assert_eq!(desktop, desktop_unverified);
        assert_ne!(desktop, mallory);
        assert_eq!(bob, bob.clone());
        assert_ne!(bob, carol);
        assert_eq!(desktop.to_string(), "alice");
        let debug = format!("{desktop:?}");
        assert!(debug.starts_with("DirectRef { nick: Nick"));
        assert!(debug.contains("\"alice\""));

        assert_ne!(direct_hash(&desktop), direct_hash(&phone));
        assert_eq!(direct_hash(&desktop), direct_hash(&desktop_unverified));
        assert_ne!(direct_hash(&desktop), direct_hash(&mallory));
        assert_ne!(direct_hash(&bob), direct_hash(&carol));
    }

    #[test]
    #[allow(clippy::mutable_key_type)]
    fn direct_references_distribute_in_hash_maps() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!p@localhost JOIN #release");
        for index in 0..64 {
            session.handle_line(
                now,
                format!(":reviewer{index:02}!review@cloak.example JOIN #release").as_bytes(),
            );
        }
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let handles: Vec<_> = (0..64)
            .map(|index| {
                let nick = session
                    .isupport()
                    .casemapping()
                    .nick(format!("reviewer{index:02}"));
                let recipient =
                    papo_core::RecipientGuard::__new(session.state(), nick, Account::Unknown);
                super::DirectRef::new(&client, recipient, false)
            })
            .collect();
        let hashes: HashSet<_> = handles.iter().map(direct_hash).collect();
        let by_recipient: HashMap<_, _> = handles
            .iter()
            .enumerate()
            .map(|(index, direct)| (direct.clone(), index))
            .collect();

        assert_eq!(hashes.len(), 64);
        assert_eq!(by_recipient.len(), 64);
        for (index, direct) in handles.iter().enumerate() {
            assert_eq!(by_recipient.get(direct), Some(&index));
        }
    }

    #[tokio::test]
    async fn own_direct_reference_does_not_reserve_monitor() {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 001 papo :Welcome to ErgoTest",
        );
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);

        let direct = client.direct("papo").await.unwrap();
        assert_eq!(direct.to_string(), "papo");
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn known_unshared_message_peer_reserves_monitor() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
        );
        session.monitor_add(["alice"]);
        session.handle_line(now, b":ergo.test 730 papo :alice!~alice@cloak.example");
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let actor = crate::__testing::user_source(
            session.isupport().casemapping().nick("alice"),
            Some("~alice".into()),
            Some("cloak.example".into()),
        );
        let mut message = crate::__testing::message(Some(actor), "papo", "release approved", true);
        crate::__testing::set_message_account(
            &mut message,
            Account::LoggedIn(session.isupport().casemapping().account("alice-account")),
        );
        let direct = tokio::spawn({
            let client = client.clone();
            async move { client.direct_to(&message).await }
        });

        let command = tokio::time::timeout(Duration::from_millis(50), commands.recv())
            .await
            .expect("direct_to did not reserve MONITOR")
            .unwrap();
        let crate::driver::Command::DirectAcquire { nick, result, .. } = command else {
            panic!("unexpected direct_to command")
        };
        assert_eq!(nick, "alice");
        let acquired = session.__direct_acquire(
            "alice",
            Some("~alice"),
            Some("cloak.example"),
            Account::Unknown,
        );
        result.send(Ok(acquired.clone())).unwrap();
        let crate::driver::Command::Request {
            message, result, ..
        } = next_command(&mut commands).await
        else {
            panic!("expected WHO proof request")
        };
        assert_eq!(message.params(), ["alice", "%na"]);
        result
            .send(Ok(response_with(&[
                ":ergo.test 354 papo alice alice-account",
            ])))
            .unwrap();
        let crate::driver::Command::DirectProve {
            recipient: proof,
            account,
            result,
        } = next_command(&mut commands).await
        else {
            panic!("expected account-tag identity proof")
        };
        assert_eq!(proof.__nick(), "alice");
        let recipient = session.__direct_prove(&acquired, account).unwrap();
        result.send(Ok(recipient)).unwrap();
        assert_eq!(direct.await.unwrap().unwrap().to_string(), "alice");
    }

    #[test]
    #[allow(clippy::mutable_key_type)]
    fn guarded_names_and_direct_nick_follow_renames_and_departures() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            ":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
            ":papo!p@localhost JOIN #review",
            ":alice!~alice@cloak.example JOIN #review",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let channel = session.state().channel("#review").unwrap().name().clone();
        let channel_guard = session.state().__channel_guard(&channel);
        let recipient = session.__direct_acquire(
            "alice",
            Some("~alice"),
            Some("cloak.example"),
            Account::LoggedOut,
        );
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let direct = super::DirectRef::new(&client, recipient.clone(), false);
        let original_hash = direct_hash(&direct);
        let by_recipient = HashMap::from([(direct.clone(), "release owner")]);
        assert_eq!(direct.nick(), "alice");
        session.handle_line(
            now,
            b":oper!o@staff.example RENAME #review #release :Review completed",
        );
        session.handle_line(now, b":alice!~alice@cloak.example NICK alicia");
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));
        assert_eq!(direct.nick(), "alicia");
        let renamed_recipient = papo_core::RecipientGuard::__new(
            session.state(),
            session.isupport().casemapping().nick("alicia"),
            Account::Unknown,
        );
        let renamed = super::DirectRef::new(&client, renamed_recipient, false);
        assert_eq!(direct, renamed);
        assert_eq!(direct_hash(&direct), original_hash);
        assert_eq!(direct_hash(&renamed), original_hash);
        assert_eq!(by_recipient.get(&renamed), Some(&"release owner"));

        let channel_parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("#review"),
            origin: channel.origin(),
            recipient: None,
            channel: Some(Box::new(channel_guard)),
        };
        assert_eq!(
            client
                .resolve(crate::target::Role::Channel("test channel"), channel_parts)
                .unwrap(),
            "#release"
        );
        let nick_parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("alice"),
            origin: recipient.__nick().origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(&recipient)),
            channel: None,
        };
        assert_eq!(
            client
                .resolve(crate::target::Role::Nick("test nick"), nick_parts)
                .unwrap(),
            "alicia"
        );

        session.handle_line(now, b":alicia!~alice@cloak.example QUIT :Client exited");
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));
        assert_eq!(direct.nick(), "alicia");
        assert_eq!(direct.to_string(), "alicia");
        assert!(format!("{direct:?}").contains("\"alicia\""));
        assert!(matches!(
            client.resolve(
                crate::target::Role::Nick("test nick"),
                <super::DirectRef as crate::Target>::__parts(&direct),
            ),
            Err(crate::Error::TargetGone)
        ));
    }

    #[test]
    fn unknown_guarded_nickname_is_rejected() {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        let nick = session.isupport().casemapping().nick("alice");
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick.clone(),
            Account::LoggedOut,
            papo_core::ReplyToken::__unknown(nick.origin()),
        );
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("alice"),
            origin: nick.origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(&recipient)),
            channel: None,
        };

        assert!(matches!(
            client.resolve(crate::target::Role::Nick("test nick"), parts),
            Err(crate::Error::TargetGone)
        ));
    }

    #[tokio::test]
    async fn unknown_recipient_send_requires_the_same_verified_account() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let nick = session.isupport().casemapping().nick("alice");
        let account = Account::LoggedIn(session.isupport().casemapping().account("alice-account"));
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick.clone(),
            account.clone(),
            papo_core::ReplyToken::__unknown(nick.origin()),
        );
        let direct = super::DirectRef::new(&client, recipient.clone(), false);
        let resolved = tokio::spawn({
            let client = client.clone();
            async move { client.privmsg(direct, "release approved").await }
        });

        let crate::driver::Command::DirectAcquire { result, .. } =
            next_command(&mut commands).await
        else {
            panic!("expected MONITOR acquisition")
        };
        let acquired = session.__direct_acquire("alice", None, None, account.clone());
        result.send(Ok(acquired)).unwrap();
        let crate::driver::Command::Request {
            message, result, ..
        } = next_command(&mut commands).await
        else {
            panic!("expected WHO request")
        };
        assert_eq!(message.params(), ["alice", "%na"]);
        result
            .send(Ok(response_with(&[
                ":ergo.test 354 papo alice alice-account",
            ])))
            .unwrap();
        let crate::driver::Command::DirectProve {
            recipient: proof,
            account: refreshed,
            result,
            ..
        } = next_command(&mut commands).await
        else {
            panic!("expected verified recipient refresh")
        };
        assert_eq!(refreshed, account);
        assert_eq!(proof.__account(), &Account::Unknown);
        result.send(Ok(recipient)).unwrap();
        assert!(matches!(
            next_command(&mut commands).await,
            crate::driver::Command::Send { .. }
        ));
        assert!(resolved.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn unknown_current_account_requires_who_proof() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
        );
        session.handle_line(now, b":papo!~papo@localhost JOIN #review");
        session.handle_line(now, b":alice!~alice@cloak.example JOIN #review");
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let nick = session.isupport().casemapping().nick("alice");
        let account = Account::LoggedIn(session.isupport().casemapping().account("alice-account"));
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick.clone(),
            account.clone(),
            papo_core::ReplyToken::__unknown(nick.origin()),
        );
        let direct = super::DirectRef::new(&client, recipient, false);
        let sending = tokio::spawn({
            let client = client.clone();
            async move { client.privmsg(direct, "release approved").await }
        });

        let crate::driver::Command::DirectAcquire { result, .. } =
            next_command(&mut commands).await
        else {
            panic!("expected MONITOR acquisition")
        };
        result
            .send(Ok(session.__direct_acquire("alice", None, None, account)))
            .unwrap();
        let crate::driver::Command::Request { message, .. } = next_command(&mut commands).await
        else {
            panic!("expected WHO request")
        };
        assert_eq!(message.params(), ["alice", "%na"]);
        sending.abort();
    }

    #[tokio::test]
    async fn different_known_account_refuses_without_who() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported",
        );
        session.handle_line(now, b":papo!~papo@localhost JOIN #review * :Papo");
        session.handle_line(
            now,
            b":alice!~alice@cloak.example JOIN #review mallory-account :Alice Example",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let nick = session.isupport().casemapping().nick("alice");
        let recipient = papo_core::RecipientGuard::__from_parts(
            nick.clone(),
            Account::LoggedIn(session.isupport().casemapping().account("alice-account")),
            papo_core::ReplyToken::__unknown(nick.origin()),
        );
        let direct = super::DirectRef::new(&client, recipient, false);

        assert!(matches!(
            client.privmsg(direct, "release approved").await,
            Err(crate::Error::TargetGone)
        ));
        assert!(matches!(
            commands.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn peer_keys_compare_by_scope_and_canonical_account() {
        let account = Account::LoggedIn(Casemapping::Rfc1459.account("Alice[Ops]"));
        let first = crate::peer_key::PeerKey::historical(
            Some(&crate::peer_key::PeerScope::server("irc.example")),
            &account,
        )
        .unwrap();
        let same = crate::peer_key::PeerKey::historical(
            Some(&crate::peer_key::PeerScope::server("irc.example")),
            &account,
        )
        .unwrap();
        let other = crate::peer_key::PeerKey::historical(
            Some(&crate::peer_key::PeerScope::server("chat.example")),
            &account,
        )
        .unwrap();

        assert!(first.is_verified());
        assert_eq!(first, same);
        assert_ne!(first, other);
    }

    #[test]
    fn unverified_history_has_no_peer_key() {
        assert_eq!(
            crate::peer_key::PeerKey::historical(None, &Account::LoggedOut),
            None
        );
    }

    #[test]
    fn verified_peer_keys_round_trip_with_delimiters_and_unicode() {
        let account = Account::LoggedIn(Casemapping::Rfc1459.account("Alice[Ops]"));
        let key = crate::peer_key::PeerKey::historical(
            Some(&crate::peer_key::PeerScope::server("irc.example:🌐")),
            &account,
        )
        .unwrap();
        let encoded = key.as_verified_str().unwrap();
        let parsed = encoded.parse::<crate::peer_key::PeerKey>().unwrap();

        assert_eq!(parsed, key);
        assert_eq!(parsed.as_verified_str(), Some(encoded));
    }

    #[test]
    fn persisted_peer_accounts_are_canonicalized() {
        let key = "papo-peer-v1:rfc1459:4:ErgoAlice[Ops]"
            .parse::<crate::peer_key::PeerKey>()
            .unwrap();
        assert_eq!(
            key.as_verified_str(),
            Some("papo-peer-v1:rfc1459:4:Ergoalice{ops}")
        );
    }

    #[test]
    fn malformed_persisted_peer_keys_are_rejected() {
        use crate::peer_key::{InvalidPeerKey, PeerKey};

        for value in [
            "",
            "papo-peer-v2:rfc1459:4:Ergoalice",
            "papo-peer-v1:unknown:4:Ergoalice",
            "papo-peer-v1:rfc1459:x:Ergoalice",
            "papo-peer-v1:rfc1459:40:Ergoalice",
            "papo-peer-v1:rfc1459:0:alice",
            "papo-peer-v1:rfc1459:4:Ergo",
            "papo-peer-v1:rfc1459:4:Ergo*",
        ] {
            assert_eq!(value.parse::<PeerKey>(), Err(InvalidPeerKey), "{value}");
        }
    }
}
