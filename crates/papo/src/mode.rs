use std::future::IntoFuture;

use papo_core::ResponseMatcher;
use papo_proto::{Isupport, Mask, Message, ModeKind, Target};

use crate::{Client, Error, InvalidInput, RequestFuture};

/// Builds one or more validated mode-change commands.
pub struct SetModes {
    client: Client,
    target: Target,
    changes: Vec<Change>,
}

impl SetModes {
    /// Adds a mode that takes no argument.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, mode: char) -> Self {
        self.change(true, mode, None)
    }

    /// Removes a mode that takes no argument.
    #[must_use]
    pub fn remove(self, mode: char) -> Self {
        self.change(false, mode, None)
    }

    /// Adds a mode with its required argument.
    #[must_use]
    pub fn add_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.change(true, mode, Some(Argument::Raw(argument.into())))
    }

    /// Removes a mode with its required argument.
    #[must_use]
    pub fn remove_with(self, mode: char, argument: impl Into<String>) -> Self {
        self.change(false, mode, Some(Argument::Raw(argument.into())))
    }

    /// Adds `mask` to the channel ban list.
    #[must_use]
    pub fn ban(self, mask: impl Into<Mask>) -> Self {
        self.change(true, 'b', Some(Argument::Mask(mask.into())))
    }

    /// Removes `mask` from the channel ban list.
    #[must_use]
    pub fn unban(self, mask: impl Into<Mask>) -> Self {
        self.change(false, 'b', Some(Argument::Mask(mask.into())))
    }

    fn change(mut self, set: bool, mode: char, arg: Option<Argument>) -> Self {
        self.changes.push(Change { set, mode, arg });
        self
    }
}

impl IntoFuture for SetModes {
    type Output = Result<(), Error>;
    type IntoFuture = RequestFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        RequestFuture::new(async move {
            let messages = encode(self.client.state().isupport(), &self.target, &self.changes)?;
            for message in messages {
                let response = self
                    .client
                    .correlated(
                        message,
                        Some(ResponseMatcher::Mode(self.target.to_string())),
                    )
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
    pub fn mode(&self, target: impl Into<Target>) -> SetModes {
        SetModes {
            client: self.clone(),
            target: target.into(),
            changes: Vec::new(),
        }
    }
}

pub(crate) struct Change {
    pub(crate) set: bool,
    mode: char,
    arg: Option<Argument>,
}

enum Argument {
    Raw(String),
    Mask(Mask),
}

impl Argument {
    fn text(&self) -> &str {
        match self {
            Self::Raw(value) => value,
            Self::Mask(value) => value,
        }
    }

    fn check(&self, _isupport: &Isupport) -> Result<(), Error> {
        match self {
            Self::Raw(_) => Ok(()),
            Self::Mask(value) => value.check().map_err(|error| invalid(error.to_string())),
        }
    }
}

pub(crate) fn encode(
    isupport: &Isupport,
    target: &Target,
    changes: &[Change],
) -> Result<Vec<Message>, Error> {
    target
        .check(isupport)
        .map_err(|error| invalid(error.to_string()))?;
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
            Ok(Message::new("MODE", params))
        })
        .collect()
}

fn validate(isupport: &Isupport, change: &Change) -> Result<char, Error> {
    let mode = change.mode;
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

#[cfg(test)]
mod tests {
    use papo_proto::{Isupport, MessageRef, Target};

    use super::{Argument, Change, encode};

    fn isupport() -> Isupport {
        let mut support = Isupport::default();
        support.apply(
            &MessageRef::parse(":ergo.test 005 papo PREFIX=(qaohv)~&@%+ CHANMODES=beI,k,l,imnst MODES=2 :supported")
                .unwrap(),
        );
        support
    }

    #[test]
    fn validates_and_chunks_advertised_modes() {
        let changes = [
            Change {
                set: true,
                mode: 'o',
                arg: Some(Argument::Raw("alice".into())),
            },
            Change {
                set: true,
                mode: 'v',
                arg: Some(Argument::Raw("carol".into())),
            },
            Change {
                set: false,
                mode: 'm',
                arg: None,
            },
        ];
        let lines: Vec<_> = encode(&isupport(), &Target::from("#papo"), &changes)
            .unwrap()
            .into_iter()
            .map(|message| message.to_line().unwrap().trim_end().to_owned())
            .collect();
        assert_eq!(lines, ["MODE #papo +ov alice carol", "MODE #papo -m"]);
    }

    #[test]
    fn rejects_empty_unknown_and_mismatched_modes() {
        let support = isupport();
        let target = Target::from("#papo");
        assert!(encode(&support, &target, &[]).is_err());
        for change in [
            Change {
                set: true,
                mode: 'x',
                arg: None,
            },
            Change {
                set: true,
                mode: 'k',
                arg: None,
            },
            Change {
                set: false,
                mode: 'l',
                arg: Some(Argument::Raw("20".into())),
            },
            Change {
                set: true,
                mode: 'b',
                arg: Some(Argument::Mask(":*!*@spam.example".into())),
            },
        ] {
            assert!(encode(&support, &target, &[change]).is_err());
        }
    }
}
