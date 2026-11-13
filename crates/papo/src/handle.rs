use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex, OnceLock, RwLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum UserIdentity {
    Expected(crate::Origin, String),
    Proven(papo_core::ReplyToken),
}

#[derive(Default)]
pub(crate) struct WatchRegistry {
    entries: Mutex<HashMap<UserIdentity, WatchEntry>>,
    commands: Mutex<Option<tokio::sync::mpsc::WeakSender<crate::driver::Command>>>,
    next_order: AtomicU64,
}

impl std::fmt::Debug for WatchRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WatchRegistry { .. }")
    }
}

struct WatchEntry {
    user: Weak<UserInner>,
    leases: usize,
    live: bool,
    order: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl WatchRegistry {
    pub(crate) fn set_commands(
        &self,
        commands: tokio::sync::mpsc::WeakSender<crate::driver::Command>,
    ) {
        *lock(&self.commands) = Some(commands);
    }

    fn acquire(self: &Arc<Self>, user: &UserHandle) -> Watch {
        let identity = user.0.identity.clone();
        let mut entries = lock(&self.entries);
        let retained = if let Some(entry) = entries.get_mut(&identity) {
            entry.leases = entry.leases.saturating_add(1);
            entry.user.upgrade().unwrap_or_else(|| Arc::clone(&user.0))
        } else {
            entries.insert(
                identity.clone(),
                WatchEntry {
                    user: Arc::downgrade(&user.0),
                    leases: 1,
                    live: false,
                    order: self.next_order.fetch_add(1, Ordering::Relaxed),
                },
            );
            Arc::clone(&user.0)
        };
        drop(entries);
        self.wake();
        Watch(Arc::new(WatchLease {
            registry: Arc::clone(self),
            identity,
            _user: retained,
        }))
    }

    pub(crate) fn desired(&self) -> Vec<String> {
        let mut desired: Vec<_> = lock(&self.entries)
            .values()
            .filter_map(|entry| Some((entry.order, entry.user.upgrade()?)))
            .collect();
        desired.sort_unstable_by_key(|(order, _)| *order);
        desired
            .into_iter()
            .map(|(_, user)| UserHandle(user).nick().to_string())
            .collect()
    }

    pub(crate) fn update_live(&self, state: &crate::State) {
        let monitored: Vec<_> = state.monitored().map(ToString::to_string).collect();
        for entry in lock(&self.entries).values_mut() {
            entry.live = entry.user.upgrade().is_some_and(|user| {
                let nick = UserHandle(user).nick();
                monitored
                    .iter()
                    .any(|known| state.same_name(known, nick.as_str()))
            });
        }
    }

    fn wake(&self) {
        let commands = lock(&self.commands);
        if let Some(commands) = commands
            .as_ref()
            .and_then(tokio::sync::mpsc::WeakSender::upgrade)
        {
            _ = commands.try_send(crate::driver::Command::Composite {
                task: Box::pin(async {}),
            });
        }
    }
}

/// A synchronous lease on one desired server-side presence watch.
#[derive(Clone)]
pub struct Watch(Arc<WatchLease>);

struct WatchLease {
    registry: Arc<WatchRegistry>,
    identity: UserIdentity,
    _user: Arc<UserInner>,
}

impl Watch {
    /// Reports whether the server currently accepted this watch.
    #[must_use]
    pub fn is_live(&self) -> bool {
        lock(&self.0.registry.entries)
            .get(&self.0.identity)
            .is_some_and(|entry| entry.live)
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("Watch", formatter)
    }
}

impl PartialEq for Watch {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Watch {}

impl Drop for WatchLease {
    fn drop(&mut self) {
        let mut entries = lock(&self.registry.entries);
        if let Some(entry) = entries.get_mut(&self.identity) {
            entry.leases = entry.leases.saturating_sub(1);
            if entry.leases == 0 {
                entries.remove(&self.identity);
            }
        }
        drop(entries);
        self.registry.wake();
    }
}

/// A live identity for one IRC user.
///
/// For a handle carried by an event, read mutable user facts from that event's paired
/// [`crate::State`] with [`crate::State::user`].
#[derive(Clone)]
pub struct UserHandle(Arc<UserInner>);

#[allow(dead_code)]
struct UserInner {
    identity: UserIdentity,
    nick: crate::Nick,
    account: crate::Account,
    proof: OnceLock<papo_core::Proven>,
    state: Option<Arc<RwLock<Arc<crate::State>>>>,
    historical: bool,
}

#[allow(dead_code)]
impl UserHandle {
    pub(crate) fn from_nick(nick: crate::Nick) -> Self {
        Self::from_parts(nick, crate::Account::Unknown, None, None)
    }

    pub(crate) fn new(
        client: &crate::Client,
        nick: crate::Nick,
        account: crate::Account,
        proof: Option<papo_core::Proven>,
    ) -> Self {
        Self::from_parts(nick, account, proof, Some(&client.state))
    }

    pub(crate) fn from_parts(
        nick: crate::Nick,
        account: crate::Account,
        proof: Option<papo_core::Proven>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self::from_parts_kind(nick, account, proof, state, false)
    }

    fn from_parts_kind(
        nick: crate::Nick,
        account: crate::Account,
        proof: Option<papo_core::Proven>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
        historical: bool,
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
            state: state.map(Arc::clone),
            historical,
        }))
    }

    pub(crate) fn from_current_state(
        nick: crate::Nick,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        let Some(shared) = state else {
            return Self::from_parts(nick, crate::Account::Unknown, None, None);
        };
        let snapshot = shared
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let account = snapshot
            .user_local(&nick)
            .map_or(crate::Account::Unknown, |user| user.account().clone());
        let token = snapshot.core.__reply_token_for(nick.as_str());
        let proof = snapshot
            .core
            .__prove_event_user(token, nick.clone(), account.clone())
            .ok();
        drop(snapshot);
        Self::from_parts(nick, account, proof, Some(shared))
    }

    pub(crate) fn from_event(
        nick: crate::Nick,
        account: crate::Account,
        token: Option<papo_core::ReplyToken>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
        historical: bool,
    ) -> Self {
        let proof = state.and_then(|shared| {
            let snapshot = shared
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let token = if historical {
                token.or_else(|| {
                    let crate::Account::LoggedIn(expected) = &account else {
                        return None;
                    };
                    let user = snapshot.user_local(&nick)?;
                    (user.account().name() == Some(expected))
                        .then(|| snapshot.core.__reply_token_for(nick.as_str()))
                })
            } else {
                token.or_else(|| Some(snapshot.core.__reply_token_for(nick.as_str())))
            }?;
            snapshot
                .core
                .__prove_event_user(token, nick.clone(), account.clone())
                .ok()
                .or_else(|| {
                    (!historical && nick == *snapshot.nick())
                        .then(|| capture_proof(&snapshot, &nick, &account))
                        .flatten()
                })
        });
        Self::from_parts_kind(nick, account, proof, state, historical)
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

    pub(crate) fn origin(&self) -> crate::Origin {
        self.0.nick.origin()
    }

    pub(crate) fn is_unproven_history(&self) -> bool {
        self.0.historical && self.proof().is_none()
    }

    /// Returns the current nickname, or the last known nickname after departure.
    #[must_use]
    pub fn nick(&self) -> crate::Nick {
        let Some(state) = self.0.state.as_ref() else {
            return self.0.nick.clone();
        };
        let state = state
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
        let Some(state) = self.0.state.as_ref() else {
            return crate::PeerKey::unverified(None, &nick);
        };
        let state = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let account = if self.0.account == crate::Account::Unknown {
            state
                .user_local(&nick)
                .map_or(&self.0.account, crate::User::account)
        } else {
            &self.0.account
        };
        crate::PeerKey::live(
            state.peer_scope().as_ref(),
            &nick,
            account,
            state.peer_account_id(&nick, account),
            state.peer_token(&nick),
        )
        .unwrap_or_else(|| crate::PeerKey::unverified(state.peer_scope().as_ref(), &nick))
    }

    /// Acquires a synchronous lease requesting server-side presence tracking.
    #[must_use]
    pub fn watch(&self) -> Watch {
        let registry = self
            .0
            .state
            .as_ref()
            .map(|state| {
                state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .watch_registry()
            })
            .unwrap_or_default();
        registry.acquire(self)
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

impl std::fmt::Display for UserHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.nick().fmt(formatter)
    }
}

impl PartialEq<str> for UserHandle {
    fn eq(&self, other: &str) -> bool {
        self.nick() == other
    }
}

impl PartialEq<&str> for UserHandle {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

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

fn capture_proof(
    state: &crate::State,
    nick: &crate::Nick,
    account: &crate::Account,
) -> Option<papo_core::Proven> {
    let recipient = state.capture_recipient(nick, account.clone())?;
    state
        .core
        .__prove_event_user(recipient.__token(), nick.clone(), account.clone())
        .ok()
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
            .user_local(&nick)
            .map_or(crate::Account::Unknown, |user| user.account().clone());
        let proof = capture_proof(&state, &nick, &account);
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

    #[test]
    fn presence_unknown_without_watch_or_shared_channel() {
        let now = crate::rt::now_mono();
        let mut session = papo_core::Session::new(papo_core::Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 MONITOR=100 :are supported".as_slice(),
            b":papo!papo@localhost JOIN #release * :Papo Client".as_slice(),
            b":Alice!alice@staff.ergo.chat JOIN #release alice-account :Alice Example".as_slice(),
        ] {
            session.handle_line(now, line);
        }
        session.monitor_add(["bob", "carol"]);
        session.handle_line(now, b":ergo.test 730 papo :bob!~bob@cloak.example");
        session.handle_line(now, b":ergo.test 731 papo :carol");
        let client = crate::driver::test_support::client_with_commands(&session).0;

        for (nick, expected) in [
            ("papo", crate::Presence::Online),
            ("Alice", crate::Presence::Online),
            ("bob", crate::Presence::Online),
            ("carol", crate::Presence::Offline),
            ("dave", crate::Presence::Unknown),
        ] {
            let user = client.query(nick).unwrap();
            assert_eq!(client.state().presence(&user), Ok(expected), "{nick}");
        }
    }

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
    fn watch_debug_is_opaque_and_equality_is_per_lease() {
        let client = client_with_joined_users();
        let alice = client.query("Alice").unwrap();
        let first = alice.watch();
        let second = alice.watch();

        assert_eq!(first, first.clone());
        assert_ne!(first, second);
        assert_eq!(format!("{first:?}"), "Watch { .. }");
        assert_eq!(
            format!("{:?}", client.state().watch_registry()),
            "WatchRegistry { .. }"
        );
    }

    #[test]
    fn watch_leases_share_one_desired_identity() {
        let client = client_with_joined_users();
        let user = client.query("Alice").unwrap();
        let registry = client.state().watch_registry();
        let first = user.watch();
        let cloned = first.clone();
        let second = user.watch();

        assert_eq!(registry.desired(), ["Alice"]);
        drop(first);
        drop(cloned);
        assert_eq!(registry.desired(), ["Alice"]);
        drop(second);
        assert!(registry.desired().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn watch_refcounts_one_monitor_entry() {
        use crate::driver::test_support::{client_line, config, scripted, server_line, within};
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        crate::driver::test_support::complete_connect_monitor(&mut server, &mut events, 100).await;
        let alice = client.query("alice").unwrap();
        let first = alice.watch();
        client_line(&mut server, b"MONITOR + alice\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
        assert!(first.is_live());

        let second = alice.watch();
        drop(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        drop(second);
        client_line(&mut server, b"MONITOR - alice\r\n").await;
        within(server.shutdown()).await.unwrap();
        assert!(within(driver).await.unwrap().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn watch_full_is_not_live_and_never_fails() {
        use crate::driver::test_support::{client_line, config, scripted, server_line, within};
        use tokio::io::AsyncWriteExt as _;

        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        crate::driver::test_support::complete_connect_monitor(&mut server, &mut events, 1).await;
        let alice = client.query("alice").unwrap().watch();
        client_line(&mut server, b"MONITOR + alice\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
        assert!(alice.is_live());

        let bob = client.query("bob").unwrap().watch();
        assert!(!bob.is_live());
        drop(alice);
        client_line(&mut server, b"MONITOR - alice\r\n").await;
        client_line(&mut server, b"MONITOR + bob\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver :bob\r\n").await;
        assert!(bob.is_live());
        drop(bob);
        within(server.shutdown()).await.unwrap();
        assert!(within(driver).await.unwrap().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn numeric_734_marks_watch_not_live() {
        use crate::driver::test_support::{
            client_line, config, next_event, scripted, server_line, within,
        };
        use tokio::io::AsyncWriteExt as _;

        let (mut server, client, mut events, driver) = scripted(config());
        let driver = tokio::spawn(driver);
        crate::driver::test_support::complete_connect_monitor(&mut server, &mut events, 100).await;
        let watch = client.query("alice").unwrap().watch();
        client_line(&mut server, b"MONITOR + alice\r\n").await;
        server_line(
            &mut server,
            b":ergo.test 734 papo-driver 100 alice :Monitor list is full\r\n",
        )
        .await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(crate::Event::Warning(
                crate::event::Warning::MonitorListFull { .. }
            ))
        ));
        assert!(!watch.is_live());

        within(server.shutdown()).await.unwrap();
        assert!(within(driver).await.unwrap().is_err());
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
    fn historical_account_proof_requires_the_current_account() {
        let client = client_with_joined_users();
        let alice = client.query("Alice").unwrap();
        let historical = |account| {
            crate::UserHandle::from_event(
                alice.0.nick.clone(),
                crate::Account::LoggedIn(client.state().casemapping().account(account)),
                None,
                Some(&client.state),
                true,
            )
        };

        assert!(historical("alice-account").proof().is_some());
        assert!(historical("replacement-account").proof().is_none());
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
    fn user_handles_compare_with_folded_nick_text() {
        let client = client_with_joined_users();
        let alice = client.query("Alice").unwrap();
        let folded = client.query("Alice[ops]").unwrap();

        assert!(<crate::UserHandle as PartialEq<str>>::eq(&alice, "alice"));
        assert!(!<crate::UserHandle as PartialEq<str>>::eq(&alice, "bob"));
        assert!(<crate::UserHandle as PartialEq<&str>>::eq(
            &folded,
            &"alice{ops}"
        ));
        assert!(!<crate::UserHandle as PartialEq<&str>>::eq(
            &folded,
            &"alice[dev]"
        ));
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
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let alice = client.query("alice").unwrap();
        let channel = client.channel("#release").unwrap();

        session.handle_line(now, b":alice!alice@staff.ergo.chat QUIT :Client exited");
        session.handle_line(now, b":papo!papo@localhost PART #release :Review complete");
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));

        assert!(matches!(
            client.query(&alice),
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
