use papo_proto::{Isupport, Message, ModeKind, Target};

use crate::{Error, InvalidInput};

pub(crate) struct Change {
    pub(crate) set: bool,
    pub(crate) mode: char,
    pub(crate) arg: Option<String>,
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
    for change in changes {
        validate(isupport, change)?;
    }
    let chunk = isupport.modes().unwrap_or(changes.len()).max(1);
    changes
        .chunks(chunk)
        .map(|changes| {
            let mut modes = String::new();
            let mut sign = None;
            let mut params = vec![target.to_string()];
            for change in changes {
                if sign != Some(change.set) {
                    modes.push(if change.set { '+' } else { '-' });
                    sign = Some(change.set);
                }
                modes.push(change.mode);
                params.extend(change.arg.clone());
            }
            params.insert(1, modes);
            Ok(Message::new("MODE", params))
        })
        .collect()
}

fn validate(isupport: &Isupport, change: &Change) -> Result<(), Error> {
    let prefix = isupport
        .prefix()
        .into_iter()
        .any(|prefix| prefix.mode == change.mode);
    let Some(groups) = isupport.chanmodes() else {
        return Err(invalid(format!("mode {} was not advertised", change.mode)));
    };
    let kind = if prefix {
        ModeKind::Prefix(0)
    } else if groups.list.contains(change.mode) {
        ModeKind::List
    } else if groups.always.contains(change.mode) {
        ModeKind::AlwaysArg
    } else if groups.when_set.contains(change.mode) {
        ModeKind::ArgWhenSet
    } else if groups.never.contains(change.mode) {
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
    Ok(())
}

fn invalid(message: impl Into<String>) -> Error {
    InvalidInput::InvalidMessage(message.into()).into()
}

#[cfg(test)]
mod tests {
    use papo_proto::{Isupport, MessageRef, Target};

    use super::{Change, encode};

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
                arg: Some("alice".into()),
            },
            Change {
                set: true,
                mode: 'v',
                arg: Some("carol".into()),
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
                arg: Some("20".into()),
            },
        ] {
            assert!(encode(&support, &target, &[change]).is_err());
        }
    }
}
