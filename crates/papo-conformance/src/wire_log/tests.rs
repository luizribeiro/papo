use super::redact::redact;

#[test]
fn credentials_are_redacted_and_everything_else_is_kept() {
    for (line, expected) in [
        ("AUTHENTICATE PLAIN", "AUTHENTICATE PLAIN"),
        ("AUTHENTICATE scram-sha-256", "AUTHENTICATE scram-sha-256"),
        ("AUTHENTICATE +", "AUTHENTICATE +"),
        ("AUTHENTICATE *", "AUTHENTICATE *"),
        (
            "AUTHENTICATE YWxpY2UAYWxpY2UAcTdIeC0ybVpwXzl2TGNUNHdSZkI=",
            "AUTHENTICATE <redacted, 44 bytes>",
        ),
        (
            "AUTHENTICATE cj1meWtvK2QybGJiRmdPTlJ2OXFreGRhd0wzcmZjTkhZSlkxWlZ2V1ZzN2oscz1RU1hDUitRNnNlazhiZjkyLGk9NDA5Ng==",
            "AUTHENTICATE <redacted, 96 bytes>",
        ),
        ("PASS soju-bouncer-pass", "PASS <redacted>"),
        ("IDENTIFY alice q7Hx-2mZp", "IDENTIFY <redacted> <redacted>"),
        (
            "WEBIRC gateway-pass cgiirc 192.0.2.10 192.0.2.10",
            "WEBIRC <redacted> cgiirc 192.0.2.10 192.0.2.10",
        ),
        ("OPER oper q7Hx-2mZp", "OPER oper <redacted>"),
        ("VERIFY alice 39gvdm6j8dsbm", "VERIFY alice <redacted>"),
        ("JOIN #papo,#ops k3y,0ps", "JOIN #papo,#ops <redacted>"),
        ("JOIN #papo", "JOIN #papo"),
        ("NS IDENTIFY q7Hx-2mZp", "NS IDENTIFY <redacted>"),
        (
            "NICKSERV REGISTER q7Hx-2mZp",
            "NICKSERV REGISTER <redacted>",
        ),
        ("ACCOUNT REGISTER q7Hx-2mZp", "ACCOUNT REGISTER <redacted>"),
        (
            "PRIVMSG NickServ :IDENTIFY alice q7Hx-2mZp",
            "PRIVMSG NickServ :<redacted>",
        ),
        (
            "PRIVMSG BouncerServ :user create -username alice -password q7Hx",
            "PRIVMSG BouncerServ :<redacted>",
        ),
        (
            ":NickServ!NickServ@localhost NOTICE alice :Account created",
            ":NickServ!NickServ@localhost NOTICE alice :Account created",
        ),
        (
            "@label=7 PRIVMSG #papo :release approved",
            "@label=7 PRIVMSG #papo :release approved",
        ),
        ("REGISTER alice * q7Hx-2mZp", "REGISTER alice * <redacted>"),
        (
            ":alice!alice@localhost MODE #papo +kl k3y 10",
            ":alice!alice@localhost MODE #papo +kl <redacted> <redacted>",
        ),
        ("MODE #papo +o bob", "MODE #papo +o bob"),
        (
            ":ergo.test 324 alice #papo +knt k3y",
            ":ergo.test 324 alice #papo +knt <redacted>",
        ),
        (
            ":ergo.test 324 alice #papo +nt",
            ":ergo.test 324 alice #papo +nt",
        ),
        (
            "WEBPUSH REGISTER https://push.example/ep p256dh=BCVx;auth=FPss",
            "WEBPUSH REGISTER https://push.example/ep <redacted>",
        ),
        (
            "WEBPUSH UNREGISTER https://push.example/ep",
            "WEBPUSH UNREGISTER https://push.example/ep",
        ),
        (
            "BOUNCER ADDNETWORK host=irc.example.org;pass=unique-9241",
            "BOUNCER ADDNETWORK <redacted>",
        ),
        ("BOUNCER LISTNETWORKS", "BOUNCER LISTNETWORKS"),
    ] {
        assert_eq!(redact(line.as_bytes()), expected, "{line}");
    }
}

#[test]
fn unparsable_lines_show_only_their_length() {
    assert_eq!(redact(b""), "<unparsed line, 0 bytes>");
    assert_eq!(redact(b"PASS \xff\xfesecret"), "<unparsed line, 13 bytes>");
    let oversized = format!("PRIVMSG #papo :{}", "a".repeat(512));
    assert_eq!(redact(oversized.as_bytes()), "<unparsed line, 527 bytes>");
}
