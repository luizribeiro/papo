use papo_proto::{Line, Nick};

use crate::RecipientGuard;

use super::proof::{Proven, ProvenKind};

/// A request command whose first parameter is filled from validated person proof.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonCommand {
    line: Line,
}

impl PersonCommand {
    /// Captures a request template and reserves its first parameter for the proven person.
    ///
    /// # Errors
    ///
    /// Returns [`papo_proto::BuildError::InvalidParam`] when the command has no first parameter.
    #[doc(hidden)]
    pub fn __new(line: Line) -> Result<Self, papo_proto::BuildError> {
        if line.as_ref().param(0).is_none() {
            return Err(papo_proto::BuildError::InvalidParam(0));
        }
        Ok(Self { line })
    }

    pub(crate) fn with_target(mut self, nick: &Nick) -> Line {
        let replaced = self.line.__replace_param(0, nick.to_string());
        debug_assert!(
            replaced,
            "PersonCommand construction requires parameter zero"
        );
        self.line
    }
}

impl Proven {
    pub(crate) fn captured_nick(&self) -> &Nick {
        match &self.kind {
            ProvenKind::Person(recipient) => recipient.__nick(),
            ProvenKind::CurrentName(nick) => nick,
        }
    }

    /// Returns the validated compatibility guard when this proof identifies one person.
    #[doc(hidden)]
    #[must_use]
    pub const fn __recipient(&self) -> Option<&RecipientGuard> {
        match &self.kind {
            ProvenKind::Person(recipient) => Some(recipient),
            ProvenKind::CurrentName(_) => None,
        }
    }
}
