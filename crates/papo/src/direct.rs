use std::{
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock, RwLock},
};

use crate::Nick;

/// A stable, guarded identity for direct messages to one IRC user.
///
/// The handle follows a nickname change only when papo observes that user's `NICK` line, and
/// refuses a send with [`crate::Error::TargetGone`] after the identity leaves or the nickname
/// changes hands. When the user shares no channel with the
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
    upgraded_peer_key: OnceLock<crate::peer_key::PeerKey>,
    recipient: papo_core::RecipientGuard,
    state: Arc<RwLock<Arc<crate::State>>>,
    user: Option<crate::UserHandle>,
    _watch: Option<crate::Watch>,
}

impl DirectRef {
    pub(crate) fn new(
        client: &crate::Client,
        recipient: papo_core::RecipientGuard,
        monitored: bool,
    ) -> Self {
        assert!(!monitored, "monitored references transfer an acquisition");
        Self::with_lease(client, recipient, None, None)
    }

    pub(crate) fn watched(
        client: &crate::Client,
        recipient: papo_core::RecipientGuard,
        user: crate::UserHandle,
        watch: crate::Watch,
    ) -> Self {
        Self::with_lease(client, recipient, Some(watch), Some(user))
    }

    fn with_lease(
        client: &crate::Client,
        recipient: papo_core::RecipientGuard,
        watch: Option<crate::Watch>,
        user: Option<crate::UserHandle>,
    ) -> Self {
        let identity = recipient.__token();
        let state = client.state();
        let scope = state.peer_scope();
        let peer_key = crate::peer_key::PeerKey::live(
            scope.as_ref(),
            recipient.__nick(),
            recipient.__account(),
            state.peer_account_id(recipient.__nick(), recipient.__account()),
            state.peer_token(recipient.__nick()),
        )
        .unwrap_or_else(|| {
            crate::peer_key::PeerKey::unverified(scope.as_ref(), recipient.__nick())
        });
        Self(Arc::new(DirectInner {
            _watch: watch,
            identity,
            peer_key,
            upgraded_peer_key: OnceLock::new(),
            recipient,
            state: Arc::clone(&client.state),
            user,
        }))
    }

    pub(crate) fn recipient(&self) -> &papo_core::RecipientGuard {
        &self.0.recipient
    }

    pub(crate) fn user_handle(&self) -> crate::UserHandle {
        if let Some(user) = &self.0.user {
            return user.clone();
        }
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let proof = state
            .core
            .__prove_recipient(&self.0.recipient)
            .expect("a direct reference retains validated identity proof");
        drop(state);
        crate::UserHandle::from_parts(
            self.nick(),
            self.0.recipient.__account().clone(),
            Some(proof),
            Some(&self.0.state),
        )
    }

    /// Returns the current saved key for this direct conversation.
    ///
    /// A Continuity pin captured before account proof upgrades to an Account pin once papo proves
    /// the account for the same uninterrupted user. Nicknames remain the conversation boundary
    /// even when two references have equal pins. See [`crate::PeerKey`] for the nickname-keyed
    /// thread and warning flow.
    #[must_use]
    pub fn peer_key(&self) -> &crate::PeerKey {
        self.refresh_peer_key();
        self.0.upgraded_peer_key.get().unwrap_or(&self.0.peer_key)
    }

    fn refresh_peer_key(&self) {
        if self.0.upgraded_peer_key.get().is_some() {
            return;
        }
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let papo_core::RecipientStatus::Current(nick) = state.resolve_recipient(&self.0.recipient)
        else {
            return;
        };
        let Some(user) = state.user_local(&nick) else {
            return;
        };
        if state
            .capture_recipient(&nick, crate::Account::Unknown)
            .is_none_or(|recipient| recipient.__token() != self.0.identity)
        {
            return;
        }
        let scope = state.peer_scope();
        let Some(current) = crate::peer_key::PeerKey::live(
            scope.as_ref(),
            &nick,
            user.account(),
            state.peer_account_id(&nick, user.account()),
            state.peer_token(&nick),
        ) else {
            return;
        };
        if matches!(
            self.0.peer_key.check(&current),
            crate::PeerKeyCheck::Upgrade(_)
        ) {
            _ = self.0.upgraded_peer_key.set(current);
        }
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
            channel: None,
            user: Some(self.user_handle()),
        }
    }
}

impl crate::Client {
    /// Captures the identity currently using `nick` for guarded direct messages.
    ///
    /// A user who shares no channel is watched while the returned reference lives; one
    /// `WHO nick %na` establishes the identity before this future resolves.
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
    /// Returns an unsupported, server, invalid-input, timeout, or disconnected error if the
    /// identity cannot be established without failing open.
    pub fn direct(&self, nick: impl crate::AsNick) -> crate::RequestFuture<DirectRef> {
        let nick = crate::AsNick::__parts(&nick).into_owned();
        self.direct_parts(nick)
    }

    fn direct_parts(
        &self,
        nick: crate::target::sealed::Parts<'static>,
    ) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("direct: nick"), nick)?;
            let snapshot = client.state();
            if let Some(user) = snapshot.user_local(&nick)
                && (user.nick() == snapshot.nick() || user.channels().len() != 0)
            {
                let Some(recipient) =
                    snapshot.capture_recipient(user.nick(), user.account().clone())
                else {
                    return Err(crate::Error::TargetGone);
                };
                return Ok(DirectRef::new(&client, recipient, false));
            }
            let nick = snapshot
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?
                .nick(nick);
            let handle = crate::UserHandle::new(&client, nick, crate::Account::Unknown, None);
            drop(snapshot);
            let watch = handle.watch();
            let proven = crate::identity::prove(&client, &handle).await?;
            let recipient = proven
                .core
                .__recipient()
                .cloned()
                .ok_or(crate::Error::TargetGone)?;
            Ok(DirectRef::watched(&client, recipient, handle, watch))
        })
    }

    /// Opens a guarded direct-message handle only when the current identity matches `stored`.
    ///
    /// An upgraded pin is accepted and available from the returned handle. On a mismatch, the
    /// error contains the exhaustive reason and current pin so the application can ask the user
    /// before opening a fresh handle with [`Self::direct`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::PeerPinMismatch`] without returning a handle when the stored pin
    /// does not authorize the current peer. It otherwise returns the errors documented by
    /// [`Self::direct`].
    pub fn direct_pinned(
        &self,
        nick: impl crate::AsNick,
        stored: &crate::PeerKey,
    ) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        let stored = stored.clone();
        crate::RequestFuture::new(async move {
            let current = client.direct_parts(nick).await?;
            match stored.check(current.peer_key()) {
                crate::PeerKeyCheck::Match | crate::PeerKeyCheck::Upgrade(_) => Ok(current),
                crate::PeerKeyCheck::Mismatch(reason) => Err(crate::Error::PeerPinMismatch(
                    crate::PeerPinMismatch::new(reason, current.peer_key().clone()),
                )),
            }
        })
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        hash::{DefaultHasher, Hash, Hasher},
        sync::Arc,
    };

    use crate::Account;
    use papo_core::{Config, Session};
    use papo_proto::LineRef;

    fn direct_hash(direct: &super::DirectRef) -> u64 {
        let mut hasher = DefaultHasher::new();
        direct.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn direct_references_are_nickname_arguments() {
        fn assert_as_nick<T: crate::AsNick>() {}

        assert_as_nick::<super::DirectRef>();
    }

    #[test]
    fn direct_references_use_stable_captured_identity() {
        let mut session = Session::new(Config::default().nick("papo"));
        let prove = |session: &mut Session, nick: &str, account: Account| {
            let row = match &account {
                Account::LoggedIn(name) => format!(":ergo.test 354 papo {nick} {name}"),
                _ => format!(":ergo.test 354 papo {nick} 0"),
            };
            let nick = session.isupport().casemapping().nick(nick);
            let lines = [LineRef::parse(&row).unwrap().to_owned()];
            let proof = session.__prove_who_user(nick, account, &lines).unwrap();
            proof.__recipient().unwrap().clone()
        };
        let account = Account::LoggedIn(session.isupport().casemapping().account("alice-account"));
        let desktop = prove(&mut session, "alice", account.clone());
        let desktop_unverified = session
            .state()
            .__recipient_from_event(
                desktop.__token(),
                desktop.__nick().clone(),
                Account::Unknown,
            )
            .unwrap();
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
                let recipient = session
                    .state()
                    .__capture_recipient(nick, Account::Unknown)
                    .unwrap();
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
        let recipient = session
            .state()
            .__capture_recipient(
                session.state().casemapping().nick("alice"),
                Account::Unknown,
            )
            .unwrap();
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
        let renamed_recipient = session
            .state()
            .__capture_recipient(
                session.isupport().casemapping().nick("alicia"),
                Account::Unknown,
            )
            .unwrap();
        let renamed = super::DirectRef::new(&client, renamed_recipient, false);
        assert_eq!(direct, renamed);
        assert_eq!(direct_hash(&direct), original_hash);
        assert_eq!(direct_hash(&renamed), original_hash);
        assert_eq!(by_recipient.get(&renamed), Some(&"release owner"));

        let channel_parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("#review"),
            origin: channel.origin(),
            channel: Some(Box::new(channel_guard)),
            user: None,
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
            channel: None,
            user: Some(direct.user_handle()),
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
    fn departed_guarded_nickname_is_rejected() {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        session.handle_line(crate::rt::now_mono(), b":papo!~papo@localhost JOIN #review");
        session.handle_line(
            crate::rt::now_mono(),
            b":alice!~alice@cloak.example JOIN #review",
        );
        let nick = session.isupport().casemapping().nick("alice");
        let recipient = session
            .state()
            .__capture_recipient(nick.clone(), Account::LoggedOut)
            .unwrap();
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let direct = super::DirectRef::new(&client, recipient, false);
        session.handle_line(
            crate::rt::now_mono(),
            b":alice!~alice@cloak.example QUIT :Client exited",
        );
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));
        let parts = crate::target::sealed::Parts {
            text: std::borrow::Cow::Borrowed("alice"),
            origin: nick.origin(),
            channel: None,
            user: Some(direct.user_handle()),
        };

        assert!(matches!(
            client.resolve(crate::target::Role::Nick("test nick"), parts),
            Err(crate::Error::TargetGone)
        ));
    }
}
