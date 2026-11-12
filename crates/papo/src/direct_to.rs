#![allow(private_interfaces)]

use crate::DirectRef;

mod sealed {
    pub trait Sealed {
        fn actor_recipient(&self) -> Option<crate::message_ref::RecipientGuard>;
    }
}

/// A received event whose user sender can be addressed with its captured identity.
pub trait DirectSource: sealed::Sealed {}

macro_rules! direct_sources {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $ty {
                fn actor_recipient(&self) -> Option<crate::message_ref::RecipientGuard> {
                    self.receipt.actor_recipient()
                }
            }

            impl DirectSource for $ty {}
        )+
    };
}

direct_sources!(
    crate::Message,
    crate::event::CtcpQuery,
    crate::event::CtcpReply,
    crate::event::Invited,
    crate::event::MonitorOnline,
    crate::event::Notice,
    crate::event::Reacted,
    crate::event::Redacted,
    crate::event::TagMsg,
    crate::event::TypingChanged,
);

impl crate::Client {
    /// Captures the sender identity of a received private message.
    ///
    /// This is the race-free way to answer a line's sender privately when the answer is not a
    /// semantic reply to that line.
    pub fn direct_to(&self, event: &impl DirectSource) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        let recipient = sealed::Sealed::actor_recipient(event);
        crate::RequestFuture::new(async move {
            let recipient = recipient.ok_or_else(|| {
                crate::InvalidInput::InvalidRequest("event actor is not a user".into())
            })?;
            let proven = recipient.has_proof();
            let nick = recipient.nick.to_string();
            let expected = recipient.account.clone();
            let snapshot = client.state();
            let core = recipient.to_core(&snapshot);
            if proven && core.is_none() {
                return Err(crate::Error::TargetGone);
            }
            if core.as_ref().is_some_and(|recipient| {
                matches!(
                    snapshot.resolve_recipient(recipient),
                    papo_core::RecipientStatus::Gone
                )
            }) {
                return Err(crate::Error::TargetGone);
            }
            let user = snapshot.user_local(&nick);
            let shared = user
                .is_some_and(|user| user.nick() == snapshot.nick() || user.channels().len() != 0);
            if shared
                && let Some(recipient) = core.or_else(|| {
                    user.and_then(|user| snapshot.capture_recipient(user.nick(), expected.clone()))
                })
            {
                return Ok(DirectRef::new(&client, recipient, false));
            }
            let nick = snapshot
                .known_name_rules()
                .ok_or(crate::InvalidInput::NameRulesUnknown)?
                .nick(nick);
            let handle = crate::UserHandle::new(&client, nick, expected, None);
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
