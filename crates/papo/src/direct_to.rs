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
            let proven = recipient.receipt.is_some();
            let recipient = recipient.to_core();
            let nick = recipient.__nick().to_string();
            let snapshot = client.state();
            if proven
                && matches!(
                    snapshot.resolve_recipient(&recipient),
                    papo_core::RecipientStatus::Gone
                )
            {
                return Err(crate::Error::TargetGone);
            }
            let shared = snapshot
                .user(&nick)
                .is_some_and(|user| user.nick() == snapshot.nick() || user.channels().len() != 0);
            if shared {
                return Ok(DirectRef::new(&client, recipient, false));
            }
            crate::direct_runtime::require_monitor_slot(&snapshot, &nick)?;
            drop(snapshot);
            let acquired = crate::direct_runtime::acquire(&client, nick.clone()).await?;
            if !matches!(recipient.__account(), crate::Account::LoggedIn(_)) {
                return Ok(DirectRef::new(&client, acquired, true));
            }
            let (refreshed, actual) = crate::direct_runtime::prove(&client, nick, acquired).await?;
            if !matches!(
                (recipient.__account(), &actual),
                (crate::Account::LoggedIn(expected), crate::Account::LoggedIn(actual))
                    if expected == actual
            ) {
                crate::direct_runtime::release(&client, refreshed).await;
                return Err(crate::Error::TargetGone);
            }
            Ok(DirectRef::new(&client, refreshed, true))
        })
    }
}
