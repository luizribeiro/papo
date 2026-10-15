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

#[test]
fn ranks_ergo_prefix_modes_and_preserves_toggles() {
    let support = support(&["CHANMODES=Ibe,k,fl,CEMRUimnstu", "PREFIX=(qaohv)~&@%+"]);
    assert_eq!(
        ModeChanges::parse("+ov-b", ["alice", "bob", "*!*@example.test"], &support)
            .collect::<Vec<_>>(),
        [
            change(true, 'o', Some("alice"), ModeKind::Prefix(2)),
            change(true, 'v', Some("bob"), ModeKind::Prefix(4)),
            change(false, 'b', Some("*!*@example.test"), ModeKind::List),
        ]
    );
}

#[test]
fn represents_missing_required_arguments_without_panicking() {
    let support = support(&["CHANMODES=eIbq,k,flj,CFLMPQScgimnprstuz", "PREFIX=(ov)@+"]);
    assert_eq!(
        ModeChanges::parse("+ob", ["dan"], &support).collect::<Vec<_>>(),
        [
            change(true, 'o', Some("dan"), ModeKind::Prefix(0)),
            change(true, 'b', None, ModeKind::List),
        ]
    );
}

#[test]
fn iterates_a_fifty_mode_change_without_truncation() {
    let support = support(&["CHANMODES=b,k,l,imnst", "PREFIX=(ov)@+"]);
    let masks = ["*!*@gateway/tor-sasl"; 50];
    let changes = ModeChanges::parse(
        "+bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        masks,
        &support,
    )
    .collect::<Vec<_>>();

    assert_eq!(changes.len(), 50);
    assert!(changes.iter().all(|parsed| {
        *parsed == change(true, 'b', Some("*!*@gateway/tor-sasl"), ModeKind::List)
    }));
}

fn change(set: bool, mode: char, arg: Option<&str>, kind: ModeKind) -> ModeChange<'_> {
    ModeChange {
        set,
        mode,
        arg,
        kind,
    }
}
