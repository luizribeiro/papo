use std::future::IntoFuture;

use papo_core::ResponseMatcher;
use papo_proto::{Isupport, Line, Mask, ModeKind, Nick};

use crate::{Client, Error, InvalidInput, RequestFuture};

/// Builds one or more validated mode-change commands.
pub struct SetModes {
    client: Client,
    target: String,
    changes: Vec<Change>,
}

impl SetModes {
    /// Adds a mode that takes no argument.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, mode: char) -> Self {
        self.change(true, Selector::Letter(mode), None)
    }

    /// Removes a mode that takes no argument.
    #[must_use]
    pub fn remove(self, mode: char) -> Self {
        self.change(false, Selector::Letter(mode), None)
    }

    /// Adds a mode with its required argument.
    #[must_use]
    pub fn add_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.change(
            true,
            Selector::Letter(mode),
            Some(Argument::Raw(argument.into())),
        )
    }

    /// Removes a mode with its required argument.
    #[must_use]
    pub fn remove_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.change(
            false,
            Selector::Letter(mode),
            Some(Argument::Raw(argument.into())),
        )
    }

    /// Adds `mask` to the channel ban list.
    #[must_use]
    pub fn ban(self, mask: impl AsRef<str>) -> Self {
        let mask = self.client.state().mask(mask.as_ref());
        self.change(true, Selector::Letter('b'), Some(Argument::Mask(mask)))
    }

    /// Removes `mask` from the channel ban list.
    #[must_use]
    pub fn unban(self, mask: impl AsRef<str>) -> Self {
        let mask = self.client.state().mask(mask.as_ref());
        self.change(false, Selector::Letter('b'), Some(Argument::Mask(mask)))
    }

    /// Grants channel-operator privileges to `nick`.
    #[must_use]
    pub fn op(self, nick: impl AsRef<str>) -> Self {
        self.prefix(true, 'o', nick)
    }

    /// Removes channel-operator privileges from `nick`.
    #[must_use]
    pub fn deop(self, nick: impl AsRef<str>) -> Self {
        self.prefix(false, 'o', nick)
    }

    /// Grants voice privileges to `nick`.
    #[must_use]
    pub fn voice(self, nick: impl AsRef<str>) -> Self {
        self.prefix(true, 'v', nick)
    }

    /// Removes voice privileges from `nick`.
    #[must_use]
    pub fn devoice(self, nick: impl AsRef<str>) -> Self {
        self.prefix(false, 'v', nick)
    }

    fn prefix(self, set: bool, mode: char, nick: impl AsRef<str>) -> Self {
        let nick = self.client.state().nickname(nick.as_ref());
        self.change(set, Selector::Prefix(mode), Some(Argument::Nick(nick)))
    }

    fn change(mut self, set: bool, mode: Selector, arg: Option<Argument>) -> Self {
        self.changes.push(Change { set, mode, arg });
        self
    }
}

impl IntoFuture for SetModes {
    type Output = Result<(), Error>;
    type IntoFuture = RequestFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        RequestFuture::new(async move {
            self.client
                .state()
                .parse_target(&self.target)
                .map_err(|error| invalid(error.to_string()))?;
            let messages = encode(
                self.client.state().isupport().inner(),
                &self.target,
                &self.changes,
            )?;
            for message in messages {
                let response = self
                    .client
                    .correlated(message, Some(ResponseMatcher::Mode(self.target.clone())))
                    .await?;
                crate::requests::empty(&response)?;
            }
            Ok(())
        })
    }
}

impl Client {
    /// Starts a validated mode-change request for `target`.
    #[must_use]
    pub fn mode(&self, target: impl AsRef<str>) -> SetModes {
        SetModes {
            client: self.clone(),
            target: target.as_ref().to_owned(),
            changes: Vec::new(),
        }
    }
}

pub(crate) struct Change {
    pub(crate) set: bool,
    mode: Selector,
    arg: Option<Argument>,
}

enum Selector {
    Letter(char),
    Prefix(char),
}

enum Argument {
    Raw(String),
    Nick(Nick),
    Mask(Mask),
}

impl Argument {
    fn text(&self) -> &str {
        match self {
            Self::Raw(value) => value,
            Self::Nick(value) => value,
            Self::Mask(value) => value,
        }
    }

    fn check(&self, isupport: &Isupport) -> Result<(), Error> {
        match self {
            Self::Raw(_) => Ok(()),
            Self::Nick(value) => value
                .check(isupport)
                .map_err(|error| invalid(error.to_string())),
            Self::Mask(value) => value.check().map_err(|error| invalid(error.to_string())),
        }
    }
}

pub(crate) fn encode(
    isupport: &Isupport,
    target: &str,
    changes: &[Change],
) -> Result<Vec<Line>, Error> {
    if changes.is_empty() {
        return Err(invalid("a mode request needs at least one change"));
    }
    let modes = changes
        .iter()
        .map(|change| validate(isupport, change))
        .collect::<Result<Vec<_>, _>>()?;
    let chunk = isupport.modes().unwrap_or(changes.len()).max(1);
    changes
        .chunks(chunk)
        .zip(modes.chunks(chunk))
        .map(|(changes, letters)| {
            let mut mode_string = String::new();
            let mut sign = None;
            let mut params = vec![target.to_string()];
            for (change, mode) in changes.iter().zip(letters) {
                if sign != Some(change.set) {
                    mode_string.push(if change.set { '+' } else { '-' });
                    sign = Some(change.set);
                }
                mode_string.push(*mode);
                params.extend(
                    change
                        .arg
                        .as_ref()
                        .map(|argument| argument.text().to_owned()),
                );
            }
            params.insert(1, mode_string);
            Ok(Line::new("MODE", params))
        })
        .collect()
}

fn validate(isupport: &Isupport, change: &Change) -> Result<char, Error> {
    let mode = match change.mode {
        Selector::Letter(mode) => mode,
        Selector::Prefix(mode) => isupport
            .prefix()
            .into_iter()
            .find(|prefix| prefix.mode == mode)
            .map(|prefix| prefix.mode)
            .ok_or_else(|| invalid(format!("prefix mode {mode} was not advertised")))?,
    };
    if let Some(argument) = &change.arg {
        argument.check(isupport)?;
    }
    let prefix = isupport
        .prefix()
        .into_iter()
        .any(|prefix| prefix.mode == mode);
    let Some(groups) = isupport.chanmodes() else {
        return Err(invalid(format!("mode {mode} was not advertised")));
    };
    let kind = if prefix {
        ModeKind::Prefix(0)
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
    };
    let accepts_arg = matches!(
        kind,
        ModeKind::List | ModeKind::AlwaysArg | ModeKind::Prefix(_)
    ) || kind == ModeKind::ArgWhenSet && change.set;
    if kind == ModeKind::Unknown || accepts_arg != change.arg.is_some() {
        return Err(invalid("mode has an invalid argument"));
    }
    Ok(mode)
}

fn invalid(message: impl Into<String>) -> Error {
    InvalidInput::InvalidMessage(message.into()).into()
}

#[cfg(all(test, not(browser)))]
mod tests {
    use papo_proto::{Isupport, LineRef};

    use super::{Argument, Change, Selector, encode};

    fn isupport() -> Isupport {
        let mut support = Isupport::default();
        support.apply(
            &LineRef::parse(":ergo.test 005 papo PREFIX=(qaohv)~&@%+ CHANMODES=beI,k,l,imnst MODES=2 :supported")
                .unwrap(),
        );
        support
    }

    #[test]
    fn validates_and_chunks_advertised_modes() {
        let support = isupport();
        let changes = [
            Change {
                set: true,
                mode: Selector::Prefix('o'),
                arg: Some(Argument::Nick(support.casemapping().nick("alice"))),
            },
            Change {
                set: true,
                mode: Selector::Prefix('v'),
                arg: Some(Argument::Nick(support.casemapping().nick("carol"))),
            },
            Change {
                set: false,
                mode: Selector::Letter('m'),
                arg: None,
            },
        ];
        let lines: Vec<_> = encode(&support, "#papo", &changes)
            .unwrap()
            .into_iter()
            .map(|message| message.to_line().unwrap().trim_end().to_owned())
            .collect();
        assert_eq!(lines, ["MODE #papo +ov alice carol", "MODE #papo -m"]);
    }

    #[test]
    fn rejects_empty_unknown_and_mismatched_modes() {
        let support = isupport();
        let target = "#papo";
        assert!(encode(&support, target, &[]).is_err());
        for change in [
            Change {
                set: true,
                mode: Selector::Letter('x'),
                arg: None,
            },
            Change {
                set: true,
                mode: Selector::Letter('k'),
                arg: None,
            },
            Change {
                set: false,
                mode: Selector::Letter('l'),
                arg: Some(Argument::Raw("20".into())),
            },
            Change {
                set: true,
                mode: Selector::Letter('b'),
                arg: Some(Argument::Mask(
                    support.casemapping().mask(":*!*@spam.example"),
                )),
            },
            Change {
                set: true,
                mode: Selector::Prefix('o'),
                arg: Some(Argument::Nick(support.casemapping().nick("bad,nick"))),
            },
            Change {
                set: true,
                mode: Selector::Prefix('y'),
                arg: Some(Argument::Nick(support.casemapping().nick("alice"))),
            },
        ] {
            assert!(encode(&support, target, &[change]).is_err());
        }
    }
}
