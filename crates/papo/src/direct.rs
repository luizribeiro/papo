use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
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
        Self(Arc::new(DirectInner {
            _watch: watch,
            identity,
            recipient,
            state: Arc::clone(&client.state),
            user,
        }))
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
}
