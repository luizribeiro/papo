//! Channel and user mode parsing behavior.

use papo_proto::{Isupport, ModeAction, ModeChange, ModeChanges, ModeKind};

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
            change(
                ModeAction::Add,
                'b',
                Some("*!*@example.test"),
                ModeKind::List
            ),
            change(ModeAction::Add, 'k', Some("swordfish"), ModeKind::AlwaysArg),
            change(ModeAction::Add, 'l', Some("10"), ModeKind::ArgWhenSet),
            change(ModeAction::Remove, 'k', Some("*"), ModeKind::AlwaysArg),
            change(ModeAction::Remove, 'l', None, ModeKind::ArgWhenSet),
            change(ModeAction::Remove, 'm', None, ModeKind::NoArg),
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
            change(ModeAction::Add, 'z', None, ModeKind::NoArg),
            change(ModeAction::Add, 'x', None, ModeKind::Unknown),
            change(
                ModeAction::Add,
                'k',
                Some("channel-key"),
                ModeKind::AlwaysArg
            ),
        ]
    );
}

#[test]
fn parses_user_modes_without_argument_semantics() {
    let support = Isupport::default();
    assert_eq!(
        ModeChanges::parse("+io-w", ["unused"], &support).collect::<Vec<_>>(),
        [
            change(ModeAction::Add, 'i', None, ModeKind::Unknown),
            change(ModeAction::Add, 'o', None, ModeKind::Unknown),
            change(ModeAction::Remove, 'w', None, ModeKind::Unknown),
        ]
    );
}

#[test]
fn parses_mixed_ergo_prefix_and_list_modes() {
    let support = support(&["CHANMODES=Ibe,k,fl,CEMRUimnstu", "PREFIX=(qaohv)~&@%+"]);
    assert_eq!(
        ModeChanges::parse("+ob-v", ["alice", "*!*@spam.example", "bob"], &support)
            .collect::<Vec<_>>(),
        [
            change(ModeAction::Add, 'o', Some("alice"), ModeKind::Prefix(2)),
            change(
                ModeAction::Add,
                'b',
                Some("*!*@spam.example"),
                ModeKind::List
            ),
            change(ModeAction::Remove, 'v', Some("bob"), ModeKind::Prefix(4)),
        ]
    );
}

#[test]
fn parses_key_and_limit_values() {
    let support = support(&["CHANMODES=Ibe,k,fl,CEMRUimnstu"]);
    assert_eq!(
        ModeChanges::parse("+kl", ["review-key-2026", "50"], &support).collect::<Vec<_>>(),
        [
            change(
                ModeAction::Add,
                'k',
                Some("review-key-2026"),
                ModeKind::AlwaysArg,
            ),
            change(ModeAction::Add, 'l', Some("50"), ModeKind::ArgWhenSet),
        ]
    );
}

#[test]
fn represents_missing_required_arguments_without_panicking() {
    let support = support(&["CHANMODES=eIbq,k,flj,CFLMPQScgimnprstuz", "PREFIX=(ov)@+"]);
    assert_eq!(
        ModeChanges::parse("+ob", ["dan"], &support).collect::<Vec<_>>(),
        [
            change(ModeAction::Add, 'o', Some("dan"), ModeKind::Prefix(0)),
            change(ModeAction::Add, 'b', None, ModeKind::List),
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
        *parsed
            == change(
                ModeAction::Add,
                'b',
                Some("*!*@gateway/tor-sasl"),
                ModeKind::List,
            )
    }));
}

fn change(action: ModeAction, mode: char, arg: Option<&str>, kind: ModeKind) -> ModeChange<'_> {
    ModeChange {
        action,
        mode,
        arg,
        kind,
    }
}
