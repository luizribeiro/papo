use std::{
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
};

use tokio::sync::mpsc;

use crate::{Account, Nick};

/// A stable, guarded identity for direct messages to one IRC user.
///
/// The handle follows nickname changes and refuses a send with [`crate::Error::TargetGone`] after
/// the identity leaves or the nickname changes hands. When the user shares no channel with the
/// client, papo reserves one server-side `MONITOR` entry until the last clone is dropped. Handles
/// compare by logged-in account when both accounts are known and otherwise by the tracked IRC user
/// identity. [`crate::Client::send_raw`] is the explicit escape hatch for deliberately unguarded
/// IRC lines.
#[derive(Clone)]
pub struct DirectRef(Arc<DirectInner>);

struct DirectInner {
    recipient: papo_core::RecipientGuard,
    state: Arc<RwLock<Arc<crate::State>>>,
    _lease: Option<MonitorLease>,
}

pub(crate) struct MonitorLease {
    recipient: papo_core::RecipientGuard,
    commands: mpsc::Sender<crate::driver::Command>,
}

impl Drop for MonitorLease {
    fn drop(&mut self) {
        _ = self
            .commands
            .try_send(crate::driver::Command::DirectRelease {
                recipient: self.recipient.clone(),
            });
    }
}

impl DirectRef {
    pub(crate) fn new(
        client: &crate::Client,
        recipient: papo_core::RecipientGuard,
        monitored: bool,
    ) -> Self {
        Self(Arc::new(DirectInner {
            _lease: monitored.then(|| MonitorLease {
                recipient: recipient.clone(),
                commands: client.commands.clone(),
            }),
            recipient,
            state: Arc::clone(&client.state),
        }))
    }

    pub(crate) fn recipient(&self) -> &papo_core::RecipientGuard {
        &self.0.recipient
    }

    fn current_nick(&self) -> Nick {
        let state = self
            .0
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.resolve_recipient(&self.0.recipient) {
            papo_core::RecipientStatus::Current(nick) => nick,
            papo_core::RecipientStatus::Gone | papo_core::RecipientStatus::Unknown => {
                self.0.recipient.__nick().clone()
            }
        }
    }
}

impl std::fmt::Debug for DirectRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectRef")
            .field("nick", &self.current_nick())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for DirectRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.current_nick().fmt(formatter)
    }
}

impl PartialEq for DirectRef {
    fn eq(&self, other: &Self) -> bool {
        match (self.0.recipient.__account(), other.0.recipient.__account()) {
            (Account::LoggedIn(left), Account::LoggedIn(right)) => left == right,
            _ => self.0.recipient.__token() == other.0.recipient.__token(),
        }
    }
}

impl Eq for DirectRef {}

impl Hash for DirectRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Account visibility can differ between two handles for the same tracked user. A constant
        // hash preserves the equality contract across that transition; IRC user sets are small.
        0_u8.hash(state);
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
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(self.current_nick().to_string()),
            origin: self.current_nick().origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(
                &self.0.recipient,
            )),
            channel: None,
        }
    }
}

pub(crate) fn lease(client: &crate::Client, recipient: papo_core::RecipientGuard) -> MonitorLease {
    MonitorLease {
        recipient,
        commands: client.commands.clone(),
    }
}

impl crate::Client {
    /// Captures the identity currently using `nick` for guarded direct messages.
    ///
    /// A user who shares no channel is added to `MONITOR`; one `WHO nick %na` establishes the
    /// identity before this future resolves. The monitor entry is removed when the last handle
    /// owning it is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::MonitorFull`] when the server's advertised `MONITOR` limit is already
    /// occupied. It also returns an unsupported, server, invalid-input, timeout, or disconnected
    /// error if the identity cannot be established without failing open.
    pub fn direct(&self, nick: impl crate::AsNick) -> crate::RequestFuture<DirectRef> {
        let client = self.clone();
        let nick = crate::AsNick::__parts(&nick).into_owned();
        crate::RequestFuture::new(async move {
            let nick = client.resolve(crate::target::Role::Nick("direct: nick"), nick)?;
            let snapshot = client.state();
            if let Some(user) = snapshot.user(&nick)
                && (user.nick() == snapshot.nick() || user.channels().len() != 0)
            {
                let recipient = snapshot.capture_recipient(user.nick(), user.account().clone());
                return Ok(DirectRef::new(&client, recipient, false));
            }
            crate::direct_runtime::require_monitor_slot(&snapshot, &nick)?;
            drop(snapshot);
            let recipient =
                crate::direct_runtime::command(&client, nick.clone(), None, true).await?;
            let response = client
                .correlated(
                    crate::Line::new("WHO").with_params([nick.clone(), "%na".to_owned()]),
                    Some(papo_core::ResponseMatcher::Who(nick.clone())),
                )
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    crate::direct_runtime::release(&client, recipient).await;
                    return Err(error);
                }
            };
            let account = match crate::direct_runtime::who_account(&client, &nick, &response) {
                Ok(account) => account,
                Err(error) => {
                    crate::direct_runtime::release(&client, recipient).await;
                    return Err(error);
                }
            };
            let recipient =
                crate::direct_runtime::command(&client, nick, Some(account), false).await?;
            Ok(DirectRef::new(&client, recipient, true))
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn direct_references_are_nickname_arguments() {
        fn assert_as_nick<T: crate::AsNick>() {}

        assert_as_nick::<super::DirectRef>();
    }
}
