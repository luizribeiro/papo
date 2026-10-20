//! Typed state-notification line views.

use papo_proto::{
    Account, Away, Chghost, Join, LineRef, Setname, StandardReply, StandardReplyKind,
};

#[test]
fn parses_standard_replies_with_context() {
    let fail = LineRef::parse(
        ":ergo.test FAIL CHATHISTORY INVALID_PARAMS #papo timestamp :Invalid parameters",
    )
    .unwrap();
    assert_eq!(
        StandardReply::parse(&fail),
        Some(StandardReply {
            kind: StandardReplyKind::Fail,
            command: "CHATHISTORY",
            code: "INVALID_PARAMS",
            context: vec!["#papo", "timestamp"],
            description: "Invalid parameters",
        })
    );

    for (line, kind) in [
        (
            ":ergo.test warn * rate_limited :Please slow down",
            StandardReplyKind::Warn,
        ),
        (
            ":ergo.test NoTe SETNAME REALNAME_OK :Realname changed",
            StandardReplyKind::Note,
        ),
    ] {
        let line = LineRef::parse(line).unwrap();
        let reply = StandardReply::parse(&line).unwrap();
        assert_eq!(reply.kind, kind);
        assert!(reply.context.is_empty());
    }
}

#[test]
fn rejects_malformed_standard_replies() {
    for line in [
        ":ergo.test PRIVMSG papo :FAIL WHOIS NO_SUCH_NICK",
        ":ergo.test FAIL",
        ":ergo.test FAIL WHOIS",
        ":ergo.test FAIL WHOIS NO_SUCH_NICK",
    ] {
        assert!(StandardReply::parse(&LineRef::parse(line).unwrap()).is_none());
    }
}

#[test]
fn parses_plain_and_extended_join_lines() {
    let plain = LineRef::parse(":alice!~alice@user/alice JOIN #papo").unwrap();
    assert_eq!(
        Join::parse(&plain),
        Some(Join {
            channel: "#papo",
            account: None,
            realname: None,
        })
    );

    let extended =
        LineRef::parse(":alice!~alice@user/alice JOIN #papo alice :Alice Example").unwrap();
    assert_eq!(
        Join::parse(&extended),
        Some(Join {
            channel: "#papo",
            account: Some("alice"),
            realname: Some("Alice Example"),
        })
    );

    let logged_out = LineRef::parse(":guest!~guest@192.0.2.4 JOIN #papo * :Guest User").unwrap();
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
        assert!(Join::parse(&LineRef::parse(line).unwrap()).is_none());
    }
}

#[test]
fn parses_account_login_and_logout() {
    let login = LineRef::parse(":alice!~alice@user/alice ACCOUNT alice").unwrap();
    let logout = LineRef::parse(":alice!~alice@user/alice ACCOUNT *").unwrap();
    assert_eq!(
        Account::parse(&login),
        Some(Account {
            account: Some("alice")
        })
    );
    assert_eq!(Account::parse(&logout), Some(Account { account: None }));
}

#[test]
fn parses_away_and_back_notifications() {
    let away = LineRef::parse(":alice!~alice@user/alice AWAY :out for lunch").unwrap();
    let empty = LineRef::parse(":alice!~alice@user/alice AWAY :").unwrap();
    let back = LineRef::parse(":alice!~alice@user/alice AWAY").unwrap();
    assert_eq!(
        Away::parse(&away),
        Some(Away {
            message: Some("out for lunch")
        })
    );
    assert_eq!(Away::parse(&empty), Some(Away { message: Some("") }));
    assert_eq!(Away::parse(&back), Some(Away { message: None }));
}

#[test]
fn rejects_account_and_away_wrong_commands_and_arities() {
    for line in [
        ":alice!~alice@user/alice PART #papo",
        ":alice!~alice@user/alice ACCOUNT",
        ":alice!~alice@user/alice ACCOUNT alice extra",
    ] {
        assert!(Account::parse(&LineRef::parse(line).unwrap()).is_none());
    }
    for line in [
        ":alice!~alice@user/alice PART #papo",
        ":alice!~alice@user/alice AWAY lunch extra",
    ] {
        assert!(Away::parse(&LineRef::parse(line).unwrap()).is_none());
    }
}

#[test]
fn parses_host_and_realname_changes() {
    let chghost = LineRef::parse(":alice!ident@old.example CHGHOST ~alice user/alice").unwrap();
    assert_eq!(
        Chghost::parse(&chghost),
        Some(Chghost {
            user: "~alice",
            host: "user/alice",
        })
    );

    let setname = LineRef::parse(":alice!~alice@user/alice SETNAME :Alice Liddell").unwrap();
    assert_eq!(
        Setname::parse(&setname),
        Some(Setname {
            realname: "Alice Liddell",
        })
    );
}

#[test]
fn rejects_chghost_and_setname_wrong_commands_and_arities() {
    for line in [
        ":alice!~alice@user/alice PART #papo",
        ":alice!ident@old.example CHGHOST ~alice",
        ":alice!ident@old.example CHGHOST ~alice user/alice extra",
    ] {
        assert!(Chghost::parse(&LineRef::parse(line).unwrap()).is_none());
    }
    for line in [
        ":alice!~alice@user/alice PART #papo",
        ":alice!~alice@user/alice SETNAME",
        ":alice!~alice@user/alice SETNAME Alice extra",
    ] {
        assert!(Setname::parse(&LineRef::parse(line).unwrap()).is_none());
    }
}
