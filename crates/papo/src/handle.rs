use std::{
    hash::{Hash, Hasher},
    sync::{Arc, OnceLock, RwLock},
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum UserIdentity {
    Expected(crate::Origin, String),
    Proven(papo_core::ReplyToken),
}

/// A live identity for one IRC user.
#[derive(Clone)]
pub struct UserHandle(Arc<UserInner>);

struct UserInner {
    identity: UserIdentity,
    nick: crate::Nick,
    account: crate::Account,
    proof: OnceLock<papo_core::Proven>,
    state: Arc<RwLock<Arc<crate::State>>>,
}

impl UserHandle {
    pub(crate) fn new(
        client: &crate::Client,
        nick: crate::Nick,
        account: crate::Account,
        proof: Option<papo_core::Proven>,
    ) -> Self {
        let identity = proof
            .as_ref()
            .and_then(papo_core::Proven::__recipient)
            .map_or_else(
                || UserIdentity::Expected(nick.origin(), nick.key().to_owned()),
                |recipient| UserIdentity::Proven(recipient.__token()),
            );
        let stored = OnceLock::new();
        if let Some(proof) = proof {
            stored.set(proof).expect("a new handle has no proof");
        }
        Self(Arc::new(UserInner {
            identity,
            nick,
            account,
            proof: stored,
            state: Arc::clone(&client.state),
        }))
    }

    pub(crate) fn proof(&self) -> Option<&papo_core::Proven> {
        self.0.proof.get()
    }

    pub(crate) fn remember(&self, proof: papo_core::Proven) {
        _ = self.0.proof.set(proof);
    }

    pub(crate) fn account(&self) -> &crate::Account {
        &self.0.account
    }

    /// Returns the current nickname, or the last known nickname after departure.
    #[must_use]
    pub fn nick(&self) -> crate::Nick {
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.proof()
            .and_then(|proof| state.core.__resolve_proven(proof).ok())
            .unwrap_or_else(|| self.0.nick.clone())
    }

    /// Captures the current saved key for this direct conversation.
    #[must_use]
    pub fn key(&self) -> crate::PeerKey {
        let nick = self.nick();
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let account = state
            .user(&nick)
            .map_or(&self.0.account, crate::User::account);
        crate::PeerKey::live(
            state.peer_scope().as_ref(),
            &nick,
            account,
            state.peer_account_id(&nick, account),
            state.peer_token(&nick),
        )
        .unwrap_or_else(|| crate::PeerKey::unverified(state.peer_scope().as_ref(), &nick))
    }
}

impl std::fmt::Debug for UserHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserHandle")
            .field("nick", &self.nick())
            .finish_non_exhaustive()
    }
}

impl PartialEq for UserHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.identity == other.0.identity
    }
}

impl Eq for UserHandle {}

impl Hash for UserHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.identity.hash(state);
    }
}

/// A live identity for one joined-channel lifecycle.
#[derive(Clone)]
pub struct ChannelHandle((Arc<ChannelInner>, Option<crate::Prefix>));

struct ChannelInner {
    channel: crate::ChannelName,
    guard: Option<papo_core::ChannelGuard>,
    state: Option<Arc<RwLock<Arc<crate::State>>>>,
}

impl ChannelHandle {
    pub(crate) fn new(
        client: &crate::Client,
        channel: crate::ChannelName,
        status: Option<crate::Prefix>,
        joined: bool,
    ) -> Self {
        let guard = joined.then(|| client.state().core.__channel_guard(&channel));
        Self::from_parts(channel, status, guard, Some(&client.state))
    }

    pub(crate) fn from_parts(
        channel: crate::ChannelName,
        status: Option<crate::Prefix>,
        guard: Option<papo_core::ChannelGuard>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self((
            Arc::new(ChannelInner {
                channel,
                guard,
                state: state.map(Arc::clone),
            }),
            status,
        ))
    }

    pub(crate) fn guard(&self) -> Option<&papo_core::ChannelGuard> {
        self.0.0.guard.as_ref()
    }

    /// Returns the channel's current name without a status prefix.
    #[must_use]
    pub fn name(&self) -> crate::ChannelName {
        self.0
            .0
            .state
            .as_ref()
            .and_then(|state| {
                let state = state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                self.guard().and_then(|guard| state.resolve_channel(guard))
            })
            .unwrap_or_else(|| self.0.0.channel.clone())
    }

    /// Returns the status prefix retained by this chat address.
    #[must_use]
    pub const fn status(&self) -> Option<crate::Prefix> {
        self.0.1
    }

    #[allow(dead_code)]
    pub(crate) fn origin(&self) -> crate::Origin {
        self.0.0.channel.origin()
    }
}

impl std::fmt::Debug for ChannelHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelHandle")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for ChannelHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name().fmt(formatter)
    }
}

impl PartialEq for ChannelHandle {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0.0.guard, &other.0.0.guard) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.0.0.channel == other.0.0.channel,
            _ => false,
        }
    }
}

impl Eq for ChannelHandle {}

impl PartialEq<str> for ChannelHandle {
    fn eq(&self, other: &str) -> bool {
        self.name() == other
    }
}

impl PartialEq<&str> for ChannelHandle {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl Hash for ChannelHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.0.guard.hash(state);
        if self.0.0.guard.is_none() {
            self.0.0.channel.hash(state);
        }
    }
}

impl crate::Client {
    /// Opens the current nickname holder only when it matches a saved peer key.
    pub fn open(&self, key: &crate::PeerKey) -> crate::RequestFuture<UserHandle> {
        let client = self.clone();
        let stored = key.clone();
        crate::RequestFuture::new(async move {
            let handle = client.query(stored.nick(client.origin()))?;
            _ = crate::identity::prove(&client, &handle).await?;
            let current = handle.key();
            match stored.check(&current) {
                crate::PeerKeyCheck::Match | crate::PeerKeyCheck::Upgrade(_) => Ok(handle),
                crate::PeerKeyCheck::Mismatch(reason) => Err(crate::Error::NotSamePerson(
                    crate::NotSamePerson::new(reason, current),
                )),
            }
        })
    }

    /// Creates a user handle without sending an IRC command.
    /// # Errors
    /// Returns [`crate::InvalidInput`] when name rules are unknown or reject the nickname.
    pub fn query(&self, nick: impl crate::AsNick) -> Result<UserHandle, crate::InvalidInput> {
        let parts = crate::AsNick::__parts(&nick).into_owned();
        let text = self
            .resolve(crate::target::Role::Nick("query: nick"), parts)
            .map_err(|error| match error {
                crate::Error::InvalidInput(error) => error,
                crate::Error::TargetGone => crate::InvalidInput::ForeignReference,
                _ => unreachable!("local nickname resolution only returns input errors"),
            })?;
        let state = self.state();
        let nick = state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?
            .nick(text);
        let account = state
            .user(&nick)
            .map_or(crate::Account::Unknown, |user| user.account().clone());
        let proof = state
            .capture_recipient(&nick, account.clone())
            .and_then(|recipient| {
                state
                    .core
                    .__prove_event_user(recipient.__token(), nick.clone(), account.clone())
                    .ok()
            });
        Ok(UserHandle::new(self, nick, account, proof))
    }

    /// Creates a channel handle without sending an IRC command.
    /// # Errors
    /// Returns [`crate::InvalidInput`] when name rules are unknown or reject the channel.
    pub fn channel(
        &self,
        channel: impl crate::AsChannel,
    ) -> Result<ChannelHandle, crate::InvalidInput> {
        let parts = crate::AsChannel::__parts(&channel).into_owned();
        let state = self.state();
        if parts.origin != crate::Origin::NONE && parts.origin != self.origin {
            return Err(crate::InvalidInput::ForeignReference);
        }
        let name_rules = state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?;
        let text = match parts.channel.as_deref() {
            Some(guard) => state
                .resolve_channel(guard)
                .ok_or(crate::InvalidInput::ForeignReference)?
                .into_string(),
            None => parts.text.into_owned(),
        };
        let (status, conversation) = state.parse_target_inner(&text).map_err(|reason| {
            crate::InvalidInput::invalid_argument("channel: channel", &text, reason)
        })?;
        let papo_core::Conversation::Channel(channel) = conversation else {
            let channel = name_rules.channel(&text);
            let Err(reason) = channel.check(state.isupport().inner()) else {
                unreachable!("a private conversation is not a channel");
            };
            return Err(crate::InvalidInput::invalid_argument(
                "channel: channel",
                text,
                reason,
            ));
        };
        Ok(ChannelHandle::new(
            self,
            channel.clone(),
            status,
            state.channel_local(&channel).is_some(),
        ))
    }
}

impl crate::target::sealed::Sealed for ChannelHandle {}

impl crate::AsChannel for ChannelHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(self.name().to_string()),
            origin: self.0.0.channel.origin(),
            recipient: None,
            channel: self.guard().cloned().map(Box::new),
            user: None,
        }
    }
}

impl crate::target::sealed::Sealed for UserHandle {}

impl crate::Target for UserHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(self.nick().to_string()),
            origin: self.0.nick.origin(),
            recipient: None,
            channel: None,
            user: Some(self.clone()),
        }
    }
}

impl crate::AsNick for UserHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        <Self as crate::Target>::__parts(self)
    }
}

impl crate::AsConversation for UserHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        <Self as crate::Target>::__parts(self)
    }
}

impl crate::Target for ChannelHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        let mut parts = <Self as crate::AsChannel>::__parts(self);
        if let Some(status) = self.status() {
            parts.text =
                std::borrow::Cow::Owned(format!("{}{name}", status.symbol(), name = self.name()));
        }
        parts
    }
}

impl crate::AsConversation for ChannelHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        <Self as crate::AsChannel>::__parts(self)
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{
        collections::{HashSet, hash_map::DefaultHasher},
        hash::{Hash, Hasher},
        sync::Arc,
    };

    use papo_core::{Config, Session};

    fn client_with_joined_users() -> crate::Client {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported".as_slice(),
            b":papo!papo@localhost JOIN #release * :Papo Client".as_slice(),
            b":papo!papo@localhost JOIN #operations * :Papo Client".as_slice(),
            b":Alice!alice@staff.ergo.chat JOIN #release alice-account :Alice Example".as_slice(),
            b":bob!bob@users.ergo.chat JOIN #release bob-account :Bob Example".as_slice(),
        ] {
            session.handle_line(now, line);
        }
        crate::driver::test_support::client_with_commands(&session).0
    }

    fn handle_hash(value: &impl Hash) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn query_and_channel_send_nothing() {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let user = client.query("alice").unwrap();
        let handle = client.channel("#release").unwrap();

        assert_eq!(user.nick(), "alice");
        assert_eq!(crate::Target::__parts(&user).user.as_ref(), Some(&user));
        assert_eq!(handle, handle.clone());
        assert_eq!(handle.name(), "#release");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn statusmsg_handle_reply_stays_ops_only() {
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# STATUSMSG=@+ :are supported"
                .as_slice(),
            b":papo!papo@localhost JOIN #ops * :Papo Client".as_slice(),
        ] {
            session.handle_line(crate::rt::now_mono(), line);
        }
        let client = crate::driver::test_support::client_with_commands(&session).0;
        let handle = client.channel("@#ops").unwrap();

        assert_eq!(handle.name(), "#ops");
        assert_eq!(handle.status().map(crate::Prefix::symbol), Some('@'));
        assert_eq!(crate::AsChannel::__parts(&handle).text(), "#ops");
        assert_eq!(crate::AsConversation::__parts(&handle).text(), "#ops");
        assert_eq!(crate::Target::__parts(&handle).text(), "@#ops");
        let reference = crate::MessageRef::from((handle, "RfcBTzyxY120004"));
        assert_eq!(crate::Target::__parts(&reference).text(), "@#ops");
    }

    #[test]
    fn channel_name_round_trips_as_saved_form() {
        let client = client_with_joined_users();
        let handle = client.channel("#release").unwrap();
        let saved = handle.name().into_string();
        let restored = client.channel(saved).unwrap();

        assert_eq!(restored, handle);
        assert_eq!(restored.name(), "#release");
        assert_eq!(restored.status(), None);
    }

    #[test]
    fn foreign_channel_lookup_is_typed() {
        let mut source_config = Config::default().nick("papo");
        source_config.origin = crate::Origin::__new(73);
        let mut source = Session::new(source_config);
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported".as_slice(),
            b":papo!papo@localhost JOIN #release * :Papo Client".as_slice(),
        ] {
            source.handle_line(crate::rt::now_mono(), line);
        }
        let source = crate::driver::test_support::client_with_commands(&source).0;
        let handle = source.channel("#release").unwrap();

        let mut destination_config = Config::default().nick("papo");
        destination_config.origin = crate::Origin::__new(91);
        let destination = Session::new(destination_config);
        let destination = crate::driver::test_support::client_with_commands(&destination).0;

        assert_eq!(
            destination.state().channel(&handle),
            Err(crate::ForeignReference)
        );
    }

    #[test]
    fn handles_debug_only_their_current_names() {
        let client = client_with_joined_users();
        let alice = client.query("Alice").unwrap();
        let release = client.channel("#release").unwrap();

        assert!(alice.proof().is_some());
        assert_eq!(
            format!("{alice:?}"),
            format!("UserHandle {{ nick: {:?}, .. }}", alice.nick())
        );
        assert_eq!(
            format!("{release:?}"),
            format!("ChannelHandle {{ name: {:?}, .. }}", release.name())
        );
        for debug in [format!("{alice:?}"), format!("{release:?}")] {
            assert!(!debug.contains("alice-account"));
            assert!(!debug.contains("proof"));
        }
    }

    #[test]
    fn handles_compare_and_hash_by_identity() {
        let client = client_with_joined_users();
        let alice = client.query("Alice").unwrap();
        let same_alice = client.query("alice").unwrap();
        let bob = client.query("bob").unwrap();
        for (left, right, equal) in [
            (&alice, &same_alice, true),
            (&alice, &bob, false),
            (&bob, &bob, true),
        ] {
            assert_eq!(left == right, equal);
        }
        assert_eq!(handle_hash(&alice), handle_hash(&same_alice));
        assert_ne!(handle_hash(&alice), handle_hash(&bob));
        assert_eq!(HashSet::from([alice, same_alice, bob]).len(), 2);

        let release = client.channel("#release").unwrap();
        let same_release = client.channel("#release").unwrap();
        let operations = client.channel("#operations").unwrap();
        for (left, right, equal) in [
            (&release, &same_release, true),
            (&release, &operations, false),
            (&operations, &operations, true),
        ] {
            assert_eq!(left == right, equal);
        }
        assert_eq!(handle_hash(&release), handle_hash(&same_release));
        assert_ne!(handle_hash(&release), handle_hash(&operations));
        assert_eq!(HashSet::from([release, same_release, operations]).len(), 2);
    }

    #[test]
    fn handle_factories_preserve_invalid_name_errors() {
        let client = client_with_joined_users();

        assert!(matches!(
            client.query("#release"),
            Err(crate::InvalidInput::InvalidName {
                argument: "query: nick",
                name,
                reason: crate::InvalidIdentifier::InvalidPrefix('#'),
            }) if name == "#release"
        ));
        assert!(matches!(
            client.channel("release"),
            Err(crate::InvalidInput::InvalidName {
                argument: "channel: channel",
                name,
                reason: crate::InvalidIdentifier::InvalidPrefix('r'),
            }) if name == "release"
        ));
    }

    #[test]
    fn handle_factories_type_expired_references() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported".as_slice(),
            b":papo!papo@localhost JOIN #release * :Papo Client".as_slice(),
            b":alice!alice@staff.ergo.chat JOIN #release * :Alice Example".as_slice(),
        ] {
            session.handle_line(now, line);
        }
        let alice = session.isupport().casemapping().nick("alice");
        let recipient = session
            .state()
            .__capture_recipient(alice, crate::Account::Unknown)
            .unwrap();
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let direct = crate::DirectRef::new(&client, recipient, false);
        let channel = client.channel("#release").unwrap();

        session.handle_line(now, b":alice!alice@staff.ergo.chat QUIT :Client exited");
        session.handle_line(now, b":papo!papo@localhost PART #release :Review complete");
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));

        assert!(matches!(
            client.query(&direct),
            Err(crate::InvalidInput::ForeignReference)
        ));
        assert!(matches!(
            client.channel(&channel),
            Err(crate::InvalidInput::ForeignReference)
        ));
    }

    #[tokio::test]
    async fn foreign_user_handle_fails_before_who() {
        let mut source = Config::default().nick("papo");
        source.origin = crate::Origin::__new(73);
        let mut source = Session::new(source);
        source.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        let (source, _) = crate::driver::test_support::client_with_commands(&source);
        let alice = source.query("alice").unwrap();

        let mut destination = Config::default().nick("papo");
        destination.origin = crate::Origin::__new(91);
        let destination = Session::new(destination);
        let (destination, mut commands) =
            crate::driver::test_support::client_with_commands(&destination);
        assert!(matches!(
            destination.privmsg(alice, "release approved").await,
            Err(crate::Error::ForeignReference(_))
        ));
        assert!(commands.try_recv().is_err());
    }
}
