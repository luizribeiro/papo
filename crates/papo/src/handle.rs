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

    pub(crate) fn recipient(&self) -> Option<papo_core::RecipientGuard> {
        self.proof()?.__recipient().cloned()
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
pub struct ChannelHandle(Arc<ChannelInner>);

struct ChannelInner {
    channel: crate::ChannelName,
    guard: Option<papo_core::ChannelGuard>,
    state: Arc<RwLock<Arc<crate::State>>>,
}

impl ChannelHandle {
    pub(crate) fn new(client: &crate::Client, channel: crate::ChannelName, joined: bool) -> Self {
        let guard = joined.then(|| client.state().core.__channel_guard(&channel));
        Self(Arc::new(ChannelInner {
            channel,
            guard,
            state: Arc::clone(&client.state),
        }))
    }

    pub(crate) fn guard(&self) -> Option<&papo_core::ChannelGuard> {
        self.0.guard.as_ref()
    }

    /// Returns the channel's current name without a status prefix.
    #[must_use]
    pub fn name(&self) -> crate::ChannelName {
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.guard()
            .and_then(|guard| state.resolve_channel(guard))
            .unwrap_or_else(|| self.0.channel.clone())
    }

    /// Returns the status prefix retained by this chat address.
    #[must_use]
    pub const fn status(&self) -> Option<crate::Prefix> {
        None
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

impl Hash for ChannelHandle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.guard.hash(state);
        if self.0.guard.is_none() {
            self.0.channel.hash(state);
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
        let text = self
            .resolve(crate::target::Role::Channel("channel: channel"), parts)
            .map_err(|error| match error {
                crate::Error::InvalidInput(error) => error,
                crate::Error::TargetGone => crate::InvalidInput::ForeignReference,
                _ => unreachable!("local channel resolution only returns input errors"),
            })?;
        let state = self.state();
        let channel = state
            .known_name_rules()
            .ok_or(crate::InvalidInput::NameRulesUnknown)?
            .channel(text);
        Ok(ChannelHandle::new(
            self,
            channel.clone(),
            state.channel(&channel).is_some(),
        ))
    }
}

impl crate::target::sealed::Sealed for ChannelHandle {}

impl crate::AsChannel for ChannelHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(self.name().to_string()),
            origin: self.0.channel.origin(),
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
        <Self as crate::AsChannel>::__parts(self)
    }
}

impl crate::AsConversation for ChannelHandle {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        <Self as crate::AsChannel>::__parts(self)
    }
}

#[cfg(test)]
mod tests {
    use papo_core::{Config, Session};

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
}
