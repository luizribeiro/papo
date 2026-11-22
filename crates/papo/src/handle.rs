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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum WatchIdentity {
    Name(String),
    User(UserIdentity),
}

#[derive(Default)]
pub(crate) struct WatchRegistry {
    entries: Mutex<HashMap<WatchIdentity, WatchEntry>>,
    accepted_names: Mutex<Vec<crate::Nick>>,
    commands: Mutex<Option<tokio::sync::mpsc::WeakSender<crate::driver::Command>>>,
    next_order: AtomicU64,
}

impl std::fmt::Debug for WatchRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WatchRegistry { .. }")
    }
}

struct WatchEntry {
    user: Option<Weak<UserInner>>,
    leases: usize,
    live: bool,
    order: u64,
}

impl WatchEntry {
    fn nick(&self, identity: &WatchIdentity) -> Option<String> {
        match identity {
            WatchIdentity::Name(nick) => Some(nick.clone()),
            WatchIdentity::User(_) => Some(
                UserHandle(self.user.as_ref()?.upgrade()?)
                    .nick()
                    .to_string(),
            ),
        }
    }
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
        self.acquire_target(WatchIdentity::User(user.0.identity.clone()), Some(&user.0))
    }

    fn acquire_target(
        self: &Arc<Self>,
        identity: WatchIdentity,
        user: Option<&Arc<UserInner>>,
    ) -> Watch {
        let mut entries = lock(&self.entries);
        let retained = if let Some(entry) = entries.get_mut(&identity) {
            entry.leases = entry.leases.saturating_add(1);
            entry
                .user
                .as_ref()
                .and_then(Weak::upgrade)
                .or_else(|| user.cloned())
        } else {
            entries.insert(
                identity.clone(),
                WatchEntry {
                    user: user.map(Arc::downgrade),
                    leases: 1,
                    live: false,
                    order: self.next_order.fetch_add(1, Ordering::Relaxed),
                },
            );
            user.cloned()
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
            .iter()
            .filter_map(|(identity, entry)| Some((entry.order, entry.nick(identity)?)))
            .collect();
        desired.sort_unstable_by_key(|(order, _)| *order);
        desired.into_iter().map(|(_, nick)| nick).collect()
    }

    pub(crate) fn update_live(&self, state: &crate::State) {
        let monitored: Vec<_> = state.monitored().cloned().collect();
        for (identity, entry) in lock(&self.entries).iter_mut() {
            if let WatchIdentity::User(_) = identity {
                entry.live = entry.nick(identity).is_some_and(|nick| {
                    monitored
                        .iter()
                        .any(|known| state.same_name(known.as_str(), &nick))
                });
            }
        }
        *lock(&self.accepted_names) = monitored;
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
    identity: WatchIdentity,
    _user: Option<Arc<UserInner>>,
}

impl Watch {
    /// Reports whether the server currently accepted this watch.
    #[must_use]
    pub fn is_live(&self) -> bool {
        match &self.0.identity {
            WatchIdentity::Name(nick) => lock(&self.0.registry.accepted_names)
                .iter()
                .any(|known| known == nick.as_str()),
            WatchIdentity::User(_) => lock(&self.0.registry.entries)
                .get(&self.0.identity)
                .is_some_and(|entry| entry.live),
        }
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
/// [`crate::State`] with [`crate::State::user`]. To answer a private history item, pass
/// the item to [`crate::Client::reply`]. Sending to its actor returns [`crate::Error::TargetGone`].
#[derive(Clone)]
pub struct UserHandle(Arc<UserInner>);

#[allow(dead_code)]
struct UserInner {
    identity: UserIdentity,
    nick: crate::Nick,
    account: crate::Account,
    proof: OnceLock<papo_core::Proven>,
    state: Option<Arc<RwLock<Arc<crate::State>>>>,
    history: History,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum History {
    Live,
    Archived,
    Private,
}

#[allow(dead_code)]
impl UserHandle {
    pub(crate) fn from_nick(nick: crate::Nick) -> Self {
        Self::current_name(nick, crate::Account::Unknown, None, None)
    }

    /// Builds the handle for a name that stands for its holder now: one the application named,
    /// the client's own nick, or a person papo-core proves present. A line naming a person in
    /// the past goes through [`Self::from_past_line`].
    pub(crate) fn current_name(
        nick: crate::Nick,
        account: crate::Account,
        proof: Option<papo_core::Proven>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self::from_parts_kind(nick, account, proof, state, History::Live)
    }

    /// Builds the handle for a person a line names in the past: an archived line, a topic
    /// setter, or the setter of a list-mode entry. papo never saw that line's sender live, so a
    /// send through it is refused unless the line named an account the current holder proves.
    pub(crate) fn from_past_line(nick: crate::Nick, account: crate::Account) -> Self {
        Self::from_parts_kind(nick, account, None, None, History::Archived)
    }

    fn from_parts_kind(
        nick: crate::Nick,
        account: crate::Account,
        proof: Option<papo_core::Proven>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
        history: History,
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
            history,
        }))
    }

    pub(crate) fn from_current_state(
        nick: crate::Nick,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        let Some(shared) = state else {
            return Self::current_name(nick, crate::Account::Unknown, None, None);
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
        Self::current_name(nick, account, proof, Some(shared))
    }

    /// Builds the handle for a person a live event names from the identity papo-core recorded
    /// when the event's line arrived, so a later holder of the nickname is never adopted.
    pub(crate) fn at_arrival(
        nick: crate::Nick,
        account: crate::Account,
        token: papo_core::ReplyToken,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self::from_event(nick, account, Some(token), state)
    }

    pub(crate) fn from_event(
        nick: crate::Nick,
        account: crate::Account,
        token: Option<papo_core::ReplyToken>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self::from_line(nick, account, state, History::Live, |snapshot, nick, _| {
            token.or_else(|| Some(snapshot.core.__reply_token_for(nick.as_str())))
        })
    }

    pub(crate) fn from_history(
        nick: crate::Nick,
        account: crate::Account,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self::from_line(nick, account, state, History::Archived, historical_token)
    }

    pub(crate) fn from_private_history(
        nick: crate::Nick,
        account: crate::Account,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self::from_line(nick, account, state, History::Private, historical_token)
    }

    pub(crate) fn is_private_history(&self) -> bool {
        self.0.history == History::Private
    }

    fn from_line(
        nick: crate::Nick,
        account: crate::Account,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
        history: History,
        token: impl FnOnce(
            &crate::State,
            &crate::Nick,
            &crate::Account,
        ) -> Option<papo_core::ReplyToken>,
    ) -> Self {
        let proof = state.and_then(|shared| {
            let snapshot = shared
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let token = token(&snapshot, &nick, &account)?;
            snapshot
                .core
                .__prove_event_user(token, nick.clone(), account.clone())
                .ok()
                .or_else(|| {
                    (history == History::Live && nick == *snapshot.nick())
                        .then(|| capture_proof(&snapshot, &nick, &account))
                        .flatten()
                })
        });
        Self::from_parts_kind(nick, account, proof, state, history)
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

    /// The nickname the line that created this handle named, which a mode echo or an
    /// party check must compare even after the person renames.
    pub(crate) fn reported_nick(&self) -> &crate::Nick {
        &self.0.nick
    }

    pub(crate) fn is_unproven_history(&self) -> bool {
        self.__names_a_past_line() && self.proof().is_none()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __names_a_past_line(&self) -> bool {
        self.0.history != History::Live
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
            .and_then(|proof| {
                state.core.__resolve_proven(proof).ok().or_else(|| {
                    proof
                        .__recipient()
                        .and_then(|recipient| state.core.__last_recipient_nick(recipient))
                })
            })
            .unwrap_or_else(|| self.0.nick.clone())
    }

    /// Acquires a server-side presence watch for the user this handle represents.
    ///
    /// It follows nickname changes the client observes for that user. Use [`crate::Client::watch`]
    /// to watch a fixed nickname.
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

/// A history line names the live member only through its own `account` tag: the user, host and
/// nickname of a past line are shared by anyone who held the nickname behind the same cloak.
fn historical_token(
    snapshot: &crate::State,
    nick: &crate::Nick,
    account: &crate::Account,
) -> Option<papo_core::ReplyToken> {
    let crate::Account::LoggedIn(expected) = account else {
        return None;
    };
    let user = snapshot.user_local(nick)?;
    (user.account().name() == Some(expected))
        .then(|| snapshot.core.__reply_token_for(nick.as_str()))
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
///
/// A handle names the whole channel and never carries a status sign: sending to it reaches
/// everyone. See [`crate::Conversation`] for answering ops-only lines.
#[derive(Clone)]
pub struct ChannelHandle(Arc<ChannelInner>);

struct ChannelInner {
    channel: crate::ChannelName,
    guard: Option<papo_core::ChannelGuard>,
    state: Option<Arc<RwLock<Arc<crate::State>>>>,
}

impl ChannelHandle {
    pub(crate) fn new(client: &crate::Client, channel: crate::ChannelName, joined: bool) -> Self {
        let guard = joined.then(|| client.state().core.__channel_guard(&channel));
        Self::from_parts(channel, guard, Some(&client.state))
    }

    pub(crate) fn from_parts(
        channel: crate::ChannelName,
        guard: Option<papo_core::ChannelGuard>,
        state: Option<&Arc<RwLock<Arc<crate::State>>>>,
    ) -> Self {
        Self(Arc::new(ChannelInner {
            channel,
            guard,
            state: state.map(Arc::clone),
        }))
    }

    pub(crate) fn guard(&self) -> Option<&papo_core::ChannelGuard> {
        self.0.guard.as_ref()
    }

    /// Returns the channel's current name.
    #[must_use]
    pub fn name(&self) -> crate::ChannelName {
        self.0
            .state
            .as_ref()
            .and_then(|state| {
                let state = state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                self.guard().and_then(|guard| state.resolve_channel(guard))
            })
            .unwrap_or_else(|| self.0.channel.clone())
    }

    #[allow(dead_code)]
    pub(crate) fn origin(&self) -> crate::Origin {
        self.0.channel.origin()
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
        match (&self.0.guard, &other.0.guard) {
            (Some(left), Some(right)) => left == right,
            (None, None) => self.0.channel == other.0.channel,
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
        self.0.guard.hash(state);
        if self.0.guard.is_none() {
            self.0.channel.hash(state);
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
    /// Acquires a server-side presence watch for the supplied nickname.
    ///
    /// The spelling is interpreted by this client's server. The watch stays on that name when
    /// its holder renames. Use [`UserHandle::watch`] to watch a user the client observed.
    /// An invalid nickname or unavailable monitor slot leaves the watch inactive.
    #[must_use]
    pub fn watch(&self, nick: impl AsRef<str>) -> Watch {
        self.state()
            .watch_registry()
            .acquire_target(WatchIdentity::Name(nick.as_ref().to_owned()), None)
    }

    /// Creates a channel handle without sending an IRC command.
    ///
    /// A handle names the whole channel. A status-prefixed address such as `@#ops` is a line's
    /// audience, not a channel, so it is refused; see [`crate::Conversation`] for ops-only
    /// guidance.
    /// # Errors
    /// Returns [`crate::Error::InvalidInput`] when name rules are unknown or reject the channel,
    /// including a `STATUSMSG` prefix such as `@#ops`,
    /// [`crate::Error::ForeignReference`] when `channel` belongs to another client, and
    /// [`crate::Error::TargetGone`] when `channel` is a handle to a channel this client has left.
    pub fn channel(&self, channel: impl crate::AsChannel) -> Result<ChannelHandle, crate::Error> {
        let parts = crate::AsChannel::__parts(&channel).into_owned();
        let state = self.state();
        crate::ForeignReference::require_local(parts.origin, self.origin)?;
        let name_rules = state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?;
        let text = match parts.channel.as_deref() {
            Some(guard) => state
                .resolve_channel(guard)
                .ok_or(crate::Error::TargetGone)?
                .into_string(),
            None => parts.text.into_owned(),
        };
        let (status, conversation) = state.parse_target_inner(&text).map_err(|reason| {
            crate::InvalidInput::invalid_argument("channel: channel", &text, reason)
        })?;
        if status.is_some() {
            let prefix = text.chars().next().unwrap_or_default();
            return Err(crate::InvalidInput::invalid_argument(
                "channel: channel",
                text,
                crate::InvalidIdentifier::InvalidPrefix(prefix),
            )
            .into());
        }
        let papo_core::Conversation::Channel(channel) = conversation else {
            let channel = name_rules.channel(&text);
            let Err(reason) = channel.check(state.isupport().inner()) else {
                unreachable!("a private conversation is not a channel");
            };
            return Err(
                crate::InvalidInput::invalid_argument("channel: channel", text, reason).into(),
            );
        };
        Ok(ChannelHandle::new(
            self,
            channel.clone(),
            state.channel_local(&channel).is_some(),
        ))
    }
}

impl crate::target::sealed::Sealed for ChannelHandle {}

impl crate::AsChannel for ChannelHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            send_name: None,
            text: std::borrow::Cow::Owned(self.name().to_string()),
            origin: self.0.channel.origin(),
            channel: self.guard().cloned().map(Box::new),
            user: None,
        }
    }
}

impl crate::target::sealed::Sealed for UserHandle {}

impl crate::Target for UserHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            send_name: None,
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
        <Self as crate::AsChannel>::__parts(self)
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
            ("ALICE", crate::Presence::Online),
            ("bob", crate::Presence::Online),
            ("carol", crate::Presence::Offline),
            ("dave", crate::Presence::Unknown),
        ] {
            let state = client.state();
            let name = state.name_rules().unwrap().nick(nick);
            assert_eq!(state.presence(nick), Ok(expected), "{nick}");
            assert_eq!(state.presence(&name), Ok(expected), "{nick}");
            let user = crate::UserHandle::from_current_state(name, Some(&client.state));
            assert_eq!(state.presence(&user), Ok(expected), "{nick}");
        }
        let foreign =
            crate::Origin::__new(73).__nick(client.state().working_casemapping(), "alice");
        assert_eq!(
            client.state().presence(foreign),
            Err(crate::ForeignReference)
        );
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
        let first = client.watch("Alice");
        let second = client.watch("Alice");

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
        let registry = client.state().watch_registry();
        let first = client.watch("Alice");
        let cloned = first.clone();
        let second = client.watch("Alice");

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
        let first = client.watch("alice");
        client_line(&mut server, b"MONITOR + alice\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
        assert!(first.is_live());

        let second = client.watch("alice");
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
        let alice = client.watch("alice");
        client_line(&mut server, b"MONITOR + alice\r\n").await;
        server_line(&mut server, b":ergo.test 731 papo-driver :alice\r\n").await;
        assert!(alice.is_live());

        let bob = client.watch("bob");
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
        let watch = client.watch("alice");
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
    fn channel_handle_sends_nothing() {
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            crate::rt::now_mono(),
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported",
        );
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let handle = client.channel("#release").unwrap();

        assert_eq!(handle, handle.clone());
        assert_eq!(handle.name(), "#release");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn channel_lookup_refuses_a_status_prefixed_name() {
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# STATUSMSG=~&@%+ :are supported"
                .as_slice(),
            b":papo!papo@localhost JOIN #ops * :Papo Client".as_slice(),
        ] {
            session.handle_line(crate::rt::now_mono(), line);
        }
        let client = crate::driver::test_support::client_with_commands(&session).0;

        for (text, prefix) in [
            ("@#ops", '@'),
            ("+#ops", '+'),
            ("+@#ops", '+'),
            ("~#ops", '~'),
        ] {
            let Err(crate::Error::InvalidInput(crate::InvalidInput::InvalidName {
                argument,
                name,
                reason,
            })) = client.channel(text)
            else {
                panic!("{text} must be refused as a channel");
            };
            assert_eq!(argument, "channel: channel");
            assert_eq!(name, text);
            assert_eq!(reason, crate::InvalidIdentifier::InvalidPrefix(prefix));
        }
        assert_eq!(client.channel("#ops").unwrap().name(), "#ops");
    }

    #[test]
    fn channel_name_round_trips_as_saved_form() {
        let client = client_with_joined_users();
        let handle = client.channel("#release").unwrap();
        let saved = handle.name().into_string();
        let restored = client.channel(saved).unwrap();

        assert_eq!(restored, handle);
        assert_eq!(restored.name(), "#release");
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

    fn observed_user(client: &crate::Client, nick: &str) -> crate::UserHandle {
        crate::UserHandle::from_current_state(
            client.state().name_rules().unwrap().nick(nick),
            Some(&client.state),
        )
    }

    #[test]
    fn handles_debug_only_their_current_names() {
        let client = client_with_joined_users();
        let alice = observed_user(&client, "Alice");
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
    fn historical_proof_rests_only_on_the_lines_account_tag() {
        let client = client_with_joined_users();
        let alice = observed_user(&client, "Alice");
        let mapping = client.state().working_casemapping();
        for (account, linked) in [
            (
                crate::Account::LoggedIn(mapping.account("alice-account")),
                true,
            ),
            (
                crate::Account::LoggedIn(mapping.account("replacement-account")),
                false,
            ),
            (crate::Account::LoggedOut, false),
            (crate::Account::Unknown, false),
        ] {
            let historical = crate::UserHandle::from_history(
                alice.0.nick.clone(),
                account.clone(),
                Some(&client.state),
            );

            assert_eq!(historical.proof().is_some(), linked, "{account:?}");
            assert_eq!(historical == alice, linked, "{account:?}");
        }
    }

    #[test]
    fn only_a_live_own_line_recaptures_a_failed_event_proof() {
        let client = client_with_joined_users();
        let evidence_free_token = client.state().core.__reply_token_for("carol");
        let event = |nick: &str, historical| {
            let nick = client.state().working_casemapping().nick(nick);
            if historical {
                crate::UserHandle::from_history(nick, crate::Account::Unknown, Some(&client.state))
            } else {
                crate::UserHandle::from_event(
                    nick,
                    crate::Account::Unknown,
                    Some(evidence_free_token),
                    Some(&client.state),
                )
            }
        };

        for (nick, historical, proven) in [
            ("papo", false, true),
            ("papo", true, false),
            ("bob", false, false),
            ("bob", true, false),
        ] {
            let handle = event(nick, historical);
            assert_eq!(
                handle.proof().is_some(),
                proven,
                "{nick} historical={historical}"
            );
            assert_eq!(handle.is_unproven_history(), historical, "{nick}");
            assert_eq!(handle.__names_a_past_line(), historical, "{nick}");
        }
    }

    #[test]
    fn handles_compare_and_hash_by_identity() {
        let client = client_with_joined_users();
        let alice = observed_user(&client, "Alice");
        let same_alice = observed_user(&client, "alice");
        let bob = observed_user(&client, "bob");
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
        let alice = observed_user(&client, "Alice");
        let folded = observed_user(&client, "Alice[ops]");

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
    fn channel_factory_preserves_invalid_name_errors() {
        let client = client_with_joined_users();

        assert!(matches!(
            client.channel("release"),
            Err(crate::Error::InvalidInput(crate::InvalidInput::InvalidName {
                argument: "channel: channel",
                name,
                reason: crate::InvalidIdentifier::InvalidPrefix('r'),
            })) if name == "release"
        ));
    }

    #[test]
    fn channel_factory_reports_departed_handles_as_gone() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported".as_slice(),
            b":papo!papo@localhost JOIN #release * :Papo Client".as_slice(),
        ] {
            session.handle_line(now, line);
        }
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let channel = client.channel("#release").unwrap();

        session.handle_line(now, b":papo!papo@localhost PART #release :Review complete");
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));

        assert!(matches!(
            client.channel(&channel),
            Err(crate::Error::TargetGone)
        ));
    }

    #[tokio::test]
    async fn requests_on_a_departed_channel_handle_fail_as_gone() {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        for line in [
            b":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# :are supported".as_slice(),
            b":papo!papo@localhost JOIN #release * :Papo Client".as_slice(),
        ] {
            session.handle_line(now, line);
        }
        let (client, mut commands) = crate::driver::test_support::client_with_commands(&session);
        let channel = client.channel("#release").unwrap();
        session.handle_line(
            now,
            b":ChanServ!ChanServ@services.ergo.test KICK #release papo :Flooding",
        );
        *client.state.write().unwrap() = Arc::new(crate::State::from_session(&session, true));

        let metadata = client.metadata();
        let avatar = || crate::MetadataKey::new("avatar");
        for (request, result) in [
            ("mode", client.mode(&channel).add('m').await.map(|_| ())),
            (
                "mode with a raw key",
                client
                    .mode(&channel)
                    .add_with('k', "hunter2")
                    .await
                    .map(|_| ()),
            ),
            (
                "metadata.get",
                metadata.get(&channel, [avatar()]).await.map(|_| ()),
            ),
            ("metadata.list", metadata.list(&channel).await.map(|_| ())),
            (
                "metadata.set_on",
                metadata
                    .set_on(&channel, avatar(), "https://example.org/a.png")
                    .await
                    .map(|_| ()),
            ),
            (
                "metadata.remove_on",
                metadata.remove_on(&channel, avatar()).await,
            ),
            (
                "metadata.clear_on",
                metadata.clear_on(&channel).await.map(|_| ()),
            ),
            ("metadata.sync", metadata.sync(&channel).await.map(|_| ())),
        ] {
            assert!(
                matches!(result, Err(crate::Error::TargetGone)),
                "{request} returned {result:?}"
            );
        }
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn foreign_user_handle_sends_nothing() {
        let now = crate::rt::now_mono();
        let mut source = Config::default().nick("papo");
        source.origin = crate::Origin::__new(73);
        let mut source = Session::new(source);
        source.handle_line(
            now,
            b":ergo.test 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        source.handle_line(
            now,
            b":alice!~alice@cloak.example PRIVMSG papo :release is ready",
        );
        let (source, _) = crate::driver::test_support::client_with_commands(&source);
        let alice = observed_user(&source, "alice");

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

    fn joined_session(lines: &[&str]) -> Session {
        let now = crate::rt::now_mono();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(
            now,
            b":irc.example 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        for line in lines {
            session.handle_line(now, line.as_bytes());
        }
        session
    }

    fn replay(client: &crate::Client, session: &mut Session, lines: &[&str]) {
        let now = crate::rt::now_mono();
        for line in lines {
            session.handle_line(now, line.as_bytes());
        }
        *client.state.write().unwrap() =
            std::sync::Arc::new(crate::State::from_session(session, true));
    }

    #[test]
    fn departed_handle_keeps_its_last_known_nick() {
        let mut session = joined_session(&[
            ":papo!p@localhost JOIN #review * :Papo",
            ":alice!~alice@cloak.example JOIN #review * :Alice",
        ]);
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let alice = observed_user(&client, "alice");

        replay(
            &client,
            &mut session,
            &[":alice!~alice@cloak.example NICK alicia"],
        );
        assert_eq!(alice.nick(), "alicia");
        replay(
            &client,
            &mut session,
            &[
                ":alicia!~alice@cloak.example QUIT :Client exited",
                ":alicia!~mallory@visitor.example JOIN #review * :Mallory",
            ],
        );
        assert_eq!(alice.nick(), "alicia");
        assert_eq!(alice.to_string(), "alicia");
        assert!(format!("{alice:?}").contains("\"alicia\""));
        assert_ne!(alice, observed_user(&client, "alicia"));
    }
}
