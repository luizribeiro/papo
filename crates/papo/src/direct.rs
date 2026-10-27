use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

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
}

impl DirectRef {
    fn current_nick(&self) -> Nick {
        self.0.recipient.__nick().clone()
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

impl crate::Target for DirectRef {
    fn __parts(&self) -> crate::target::sealed::Parts<'_> {
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(self.current_nick().to_string()),
            origin: self.current_nick().origin(),
            recipient: Some(crate::message_ref::RecipientGuard::from_core(
                &self.0.recipient,
            )),
        }
    }
}
