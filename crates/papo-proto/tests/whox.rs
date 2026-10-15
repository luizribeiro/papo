//! WHOX reply parsing behavior.

use papo_proto::{Isupport, MessageRef, WhoxFlags, WhoxPresence, WhoxReply};

#[test]
fn parses_real_ergo_state_query_reply() {
    let message = MessageRef::parse(
        ":irc.ergo.test 354 tester 42 #papo ~alice user/alice alice H@ alice :Alice Example",
    )
    .unwrap();
    let reply = WhoxReply::parse(&message, "%tcuhnfar,42").unwrap();

    assert_eq!(reply.token, Some("42"));
    assert_eq!(reply.channel, Some("#papo"));
    assert_eq!(reply.user, Some("~alice"));
    assert_eq!(reply.host, Some("user/alice"));
    assert_eq!(reply.nick, Some("alice"));
    assert_eq!(reply.raw_flags, Some("H@"));
    assert_eq!(reply.account, Some("alice"));
    assert_eq!(reply.realname, Some("Alice Example"));

    let mut support = Isupport::default();
    support.apply_tokens(["PREFIX=(qaohv)~&@%+", "BOT=B"]);
    assert_eq!(
        reply.flags(&support),
        Some(WhoxFlags {
            presence: Some(WhoxPresence::Here),
            oper: false,
            bot: false,
            prefixes: vec!['@'],
        })
    );
}

#[test]
fn maps_fixed_field_order_and_normalizes_placeholders() {
    let message = MessageRef::parse(
        ":irc.ergo.test 354 tester 7 * ~guest 255.255.255.255 hidden.example irc.ergo.test guest G*B+ 1 20 0 0 :Guest User",
    )
    .unwrap();
    let reply = WhoxReply::parse(&message, "%ronaldfshicut,7").unwrap();

    assert_eq!(reply.channel, None);
    assert_eq!(reply.ip, None);
    assert_eq!(reply.account, None);
    assert_eq!(reply.server, Some("irc.ergo.test"));
    assert_eq!(reply.hopcount, Some("1"));
    assert_eq!(reply.idle, Some("20"));
    assert_eq!(reply.oplevel, Some("0"));

    let mut support = Isupport::default();
    support.apply_tokens(["PREFIX=(ov)@+", "BOT=b"]);
    assert_eq!(
        reply.flags(&support),
        Some(WhoxFlags {
            presence: Some(WhoxPresence::Gone),
            oper: true,
            bot: true,
            prefixes: vec!['+'],
        })
    );
}

#[test]
fn accepts_custom_bot_mode_and_absent_flags() {
    let message = MessageRef::parse(":irc.ergo.test 354 tester bot Hb%").unwrap();
    let reply = WhoxReply::parse(&message, "%fn").unwrap();
    let mut support = Isupport::default();
    support.apply_tokens(["PREFIX=(ohv)@%+", "BOT=b"]);
    assert_eq!(reply.nick, Some("bot"));
    assert!(reply.flags(&support).unwrap().bot);
    assert_eq!(reply.flags(&support).unwrap().prefixes, vec!['%']);

    let no_flags = MessageRef::parse(":irc.ergo.test 354 tester alice").unwrap();
    assert_eq!(
        WhoxReply::parse(&no_flags, "n").unwrap().flags(&support),
        None
    );
}

#[test]
fn rejects_wrong_commands_selectors_and_field_counts() {
    for (line, fields) in [
        (
            ":irc.ergo.test 352 tester #papo ~alice host server alice H :0 Alice",
            "%tcuhnfar,42",
        ),
        (
            ":irc.ergo.test 354 tester 42 #papo ~alice host alice H@ alice",
            "%tcuhnfar,42",
        ),
        (
            ":irc.ergo.test 354 tester 42 #papo ~alice host alice H@ alice extra :Alice Example",
            "%tcuhnfar,42",
        ),
        (":irc.ergo.test 354 tester alice", ""),
        (":irc.ergo.test 354 tester alice", "%nn"),
        (":irc.ergo.test 354 tester alice", "%x"),
    ] {
        let message = MessageRef::parse(line).unwrap();
        assert!(WhoxReply::parse(&message, fields).is_none());
    }
}

#[test]
fn rejects_invalid_selectors_before_reading_reply_fields() {
    let target_only = MessageRef::parse(":irc.ergo.test 354 tester").unwrap();
    assert!(WhoxReply::parse(&target_only, "").is_none());
    assert!(WhoxReply::parse(&target_only, "%x").is_none());
}
