//! Typed state-notification message views.

use papo_proto::{Account, Away, Join, MessageRef};

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

#[test]
fn parses_account_login_and_logout() {
    let login = MessageRef::parse(":alice!~alice@user/alice ACCOUNT alice").unwrap();
    let logout = MessageRef::parse(":alice!~alice@user/alice ACCOUNT *").unwrap();
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
    let away = MessageRef::parse(":alice!~alice@user/alice AWAY :out for lunch").unwrap();
    let empty = MessageRef::parse(":alice!~alice@user/alice AWAY :").unwrap();
    let back = MessageRef::parse(":alice!~alice@user/alice AWAY").unwrap();
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
        assert!(Account::parse(&MessageRef::parse(line).unwrap()).is_none());
    }
    for line in [
        ":alice!~alice@user/alice PART #papo",
        ":alice!~alice@user/alice AWAY lunch extra",
    ] {
        assert!(Away::parse(&MessageRef::parse(line).unwrap()).is_none());
    }
}
