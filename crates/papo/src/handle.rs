use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

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
        }
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
    fn channel_handle_clones_keep_the_same_identity() {
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
}
