use crate::{ModeAction, Prefix, Secret, UserHandle};

/// One operation from an IRC mode string.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ModeChange {
    /// The mode letter.
    pub mode: char,
    /// Whether the mode was added or removed.
    pub action: ModeAction,
    /// The mode argument classified using the server's advertised rules.
    pub arg: Option<ModeArg>,
}

/// A mode argument classified using the server's advertised rules.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModeArg {
    /// A member and the channel-membership prefix changed for them.
    #[non_exhaustive]
    Member {
        /// The affected member, usable with [`crate::SetModes::op`] and the other member
        /// helpers.
        ///
        /// From a live line it follows that person's later nickname changes and refuses whoever
        /// takes the nickname after they leave. From history it proves nothing about who holds
        /// the nickname now, so member helpers refuse it with [`crate::Error::TargetGone`].
        user: UserHandle,
        /// The membership prefix corresponding to the mode.
        prefix: Prefix,
    },
    /// A mask for a list mode advertised in the first `CHANMODES` group.
    Mask(crate::Mask),
    /// A channel key (`+k`).
    Key(Secret),
    /// A value for any other mode that takes an argument.
    Value(String),
}

impl ModeChange {
    pub(crate) const fn new(mode: char, action: ModeAction, arg: Option<ModeArg>) -> Self {
        Self { mode, action, arg }
    }

    pub(crate) fn from_core(
        change: papo_core::ModeChange,
        member: impl FnOnce(crate::Nick) -> UserHandle,
    ) -> Self {
        let arg = change.arg.map(|arg| match arg {
            papo_core::ModeArg::Member { nick, prefix, .. } => ModeArg::Member {
                user: member(nick),
                prefix,
            },
            papo_core::ModeArg::Mask(mask) => ModeArg::Mask(mask),
            papo_core::ModeArg::Key(key) => ModeArg::Key(key),
            papo_core::ModeArg::Value(value) => ModeArg::Value(value),
            _ => unreachable!("new core mode arguments must be mapped"),
        });
        Self::new(change.mode, change.action, arg)
    }
}
