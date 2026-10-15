//! Channel and user mode parsing behavior.

use papo_proto::{Isupport, ModeChange, ModeChanges, ModeKind};

fn support(tokens: &[&str]) -> Isupport {
    let mut support = Isupport::default();
    support.apply_tokens(tokens);
    support
}

#[test]
fn applies_every_chanmodes_argument_rule() {
    let support = support(&["CHANMODES=Ibe,k,fl,CEMRUimnstu"]);

    let changes = ModeChanges::parse(
        "+bkl-klm",
        ["*!*@example.test", "swordfish", "10", "*"],
        &support,
    )
    .collect::<Vec<_>>();
    assert_eq!(
        changes,
        [
            change(true, 'b', Some("*!*@example.test"), ModeKind::List),
            change(true, 'k', Some("swordfish"), ModeKind::AlwaysArg),
            change(true, 'l', Some("10"), ModeKind::ArgWhenSet),
            change(false, 'k', Some("*"), ModeKind::AlwaysArg),
            change(false, 'l', None, ModeKind::ArgWhenSet),
            change(false, 'm', None, ModeKind::NoArg),
        ]
    );
}

#[test]
fn handles_solanum_no_arg_and_unknown_modes() {
    let support = support(&["CHANMODES=eIbq,k,flj,CFLMPQScgimnprstuz"]);
    let changes: Vec<_> = ModeChanges::parse("+zxk", ["channel-key"], &support).collect();

    assert_eq!(
        changes,
        [
            change(true, 'z', None, ModeKind::NoArg),
            change(true, 'x', None, ModeKind::Unknown),
            change(true, 'k', Some("channel-key"), ModeKind::AlwaysArg),
        ]
    );
}

#[test]
fn parses_user_modes_without_argument_semantics() {
    let support = Isupport::default();
    assert_eq!(
        ModeChanges::parse("+io-w", ["unused"], &support).collect::<Vec<_>>(),
        [
            change(true, 'i', None, ModeKind::Unknown),
            change(true, 'o', None, ModeKind::Unknown),
            change(false, 'w', None, ModeKind::Unknown),
        ]
    );
}

fn change(set: bool, mode: char, arg: Option<&str>, kind: ModeKind) -> ModeChange<'_> {
    ModeChange {
        set,
        mode,
        arg,
        kind,
    }
}
