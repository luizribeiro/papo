use crate::{ChanModes, Isupport, Prefix};

/// The parameter behavior of a channel mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeKind {
    /// A list mode that always takes an argument.
    List,
    /// A mode that always takes an argument.
    AlwaysArg,
    /// A mode that takes an argument only when being set.
    ArgWhenSet,
    /// A mode that never takes an argument.
    NoArg,
    /// A channel-membership mode and its visible prefix.
    Prefix(Prefix),
    /// A mode whose parameter behavior was not advertised.
    ///
    /// Unknown modes consume no argument. A server that gives one an argument can
    /// therefore make later arguments appear attached to the wrong known mode.
    Unknown,
}

/// One operation from an IRC mode string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeChange<'a> {
    /// Whether the mode is being added or removed.
    pub action: ModeAction,
    /// The mode letter.
    pub mode: char,
    /// The mode argument, when one was required and supplied.
    pub arg: Option<&'a str>,
    /// The mode's advertised parameter behavior.
    pub kind: ModeKind,
}

/// The operation applied to an IRC mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModeAction {
    /// Add or enable the mode.
    Add,
    /// Remove or disable the mode.
    Remove,
}

/// Parser for the operations in an IRC mode string.
pub struct ModeChanges;

impl ModeChanges {
    /// Parses mode changes using the server's advertised channel-mode semantics.
    ///
    /// A missing required argument is represented by `None`. When `CHANMODES` is
    /// absent, mode letters are unknown and consume no arguments, which is suitable
    /// for parsing user modes.
    #[must_use = "mode changes are parsed lazily"]
    pub fn parse<'a, I>(
        modestring: &'a str,
        args: I,
        isupport: &'a Isupport,
    ) -> impl Iterator<Item = ModeChange<'a>> + 'a
    where
        I: IntoIterator<Item = &'a str> + 'a,
        I::IntoIter: 'a,
    {
        let mut modes = modestring.chars();
        let mut args = args.into_iter();
        let chanmodes = isupport.chanmodes();
        let prefixes = chanmodes.map_or_else(Vec::new, |_| isupport.prefix());
        let mut action = ModeAction::Add;

        std::iter::from_fn(move || {
            loop {
                let mode = modes.next()?;
                match mode {
                    '+' => action = ModeAction::Add,
                    '-' => action = ModeAction::Remove,
                    _ => {
                        let kind = classify(mode, chanmodes.as_ref(), &prefixes);
                        let needs_arg = matches!(
                            kind,
                            ModeKind::List | ModeKind::AlwaysArg | ModeKind::Prefix(_)
                        ) || kind == ModeKind::ArgWhenSet
                            && action == ModeAction::Add;
                        let arg = if needs_arg { args.next() } else { None };
                        return Some(ModeChange {
                            action,
                            mode,
                            arg,
                            kind,
                        });
                    }
                }
            }
        })
    }
}

fn classify(mode: char, groups: Option<&ChanModes<'_>>, prefixes: &[Prefix]) -> ModeKind {
    let Some(groups) = groups else {
        return ModeKind::Unknown;
    };
    if let Some(prefix) = prefixes.iter().find(|prefix| prefix.mode() == mode) {
        ModeKind::Prefix(*prefix)
    } else if groups.list.contains(mode) {
        ModeKind::List
    } else if groups.always.contains(mode) {
        ModeKind::AlwaysArg
    } else if groups.when_set.contains(mode) {
        ModeKind::ArgWhenSet
    } else if groups.never.contains(mode) {
        ModeKind::NoArg
    } else {
        ModeKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::{ModeKind, classify};
    use crate::{ChanModes, Prefix};

    #[test]
    fn classifies_each_chanmodes_group_and_user_modes() {
        let groups = ChanModes {
            list: "Ibe",
            always: "k",
            when_set: "fl",
            never: "CEMRUimnstu",
        };
        assert_eq!(classify('b', Some(&groups), &[]), ModeKind::List);
        assert_eq!(classify('k', Some(&groups), &[]), ModeKind::AlwaysArg);
        assert_eq!(classify('l', Some(&groups), &[]), ModeKind::ArgWhenSet);
        assert_eq!(classify('m', Some(&groups), &[]), ModeKind::NoArg);
        assert_eq!(classify('x', Some(&groups), &[]), ModeKind::Unknown);
        assert_eq!(classify('o', None, &[]), ModeKind::Unknown);
    }

    #[test]
    fn prefix_modes_win_over_chanmodes_groups() {
        let groups = ChanModes {
            list: "o",
            always: "",
            when_set: "",
            never: "",
        };
        let prefix = Prefix::__new('o', '@');
        assert_eq!(
            classify('o', Some(&groups), &[prefix]),
            ModeKind::Prefix(prefix)
        );
    }
}
