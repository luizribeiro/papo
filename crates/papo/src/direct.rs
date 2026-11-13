use std::sync::{Arc, RwLock};

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
#[allow(dead_code)]
pub struct DirectRef(Arc<DirectInner>);

#[allow(dead_code)]
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
