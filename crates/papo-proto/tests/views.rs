//! Typed state-notification message views.

use papo_proto::{Join, MessageRef};

#[test]
fn parses_plain_and_extended_join_messages() {
    let plain = MessageRef::parse(":alice!~alice@user/alice JOIN #papo").unwrap();
    assert_eq!(
        Join::parse(&plain),
        Some(Join {
            channel: "#papo",
            account: None,
            realname: None,
        })
    );

    let extended =
        MessageRef::parse(":alice!~alice@user/alice JOIN #papo alice :Alice Example").unwrap();
    assert_eq!(
        Join::parse(&extended),
        Some(Join {
            channel: "#papo",
            account: Some("alice"),
            realname: Some("Alice Example"),
        })
    );

    let logged_out = MessageRef::parse(":guest!~guest@192.0.2.4 JOIN #papo * :Guest User").unwrap();
    assert_eq!(Join::parse(&logged_out).unwrap().account, None);
}

#[test]
fn rejects_join_wrong_commands_and_arities() {
    for line in [
        ":alice!~alice@user/alice PART #papo",
        ":alice!~alice@user/alice JOIN",
        ":alice!~alice@user/alice JOIN #papo alice",
        ":alice!~alice@user/alice JOIN #papo alice extra :Alice Example",
    ] {
        assert!(Join::parse(&MessageRef::parse(line).unwrap()).is_none());
    }
}
