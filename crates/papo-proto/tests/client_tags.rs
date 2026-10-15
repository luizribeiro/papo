//! Client-only message-tag parsing coverage.

use papo_proto::{MessageRef, Reaction, Typing};

#[test]
fn parses_all_typing_states() {
    for (value, expected) in [
        ("active", Typing::Active),
        ("paused", Typing::Paused),
        ("done", Typing::Done),
    ] {
        let line = format!("@+typing={value} :alice!~a@cloak TAGMSG #papo");
        assert_eq!(
            Typing::parse(&MessageRef::parse(&line).unwrap()),
            Some(expected)
        );
    }
    let unknown = MessageRef::parse("@+typing=idle :alice TAGMSG #papo").unwrap();
    assert_eq!(Typing::parse(&unknown), None);
}

#[test]
fn parses_react_and_unreact_with_reply_aliases() {
    for (line, react, unreact, reply) in [
        (
            "@+draft/react=👍;+reply=RKi9Cw TAGMSG #papo",
            Some("👍"),
            None,
            "RKi9Cw",
        ),
        (
            "@+draft/unreact=👍;+draft/reply=8sKY2A TAGMSG #papo",
            None,
            Some("👍"),
            "8sKY2A",
        ),
    ] {
        let message = MessageRef::parse(line).unwrap();
        let reaction = Reaction::parse(&message).unwrap();
        assert_eq!(reaction.react.as_deref(), react);
        assert_eq!(reaction.unreact.as_deref(), unreact);
        assert_eq!(reaction.reply_to, reply);
    }
}

#[test]
fn rejects_reactions_without_a_reply() {
    let message = MessageRef::parse("@+draft/react=👍 TAGMSG #papo").unwrap();
    assert_eq!(Reaction::parse(&message), None);
}
