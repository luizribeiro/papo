//! Integration tests for borrowed IRC message parsing.

use std::{borrow::Cow, collections::BTreeMap};

use papo_proto::{BuildError, Command, Message, MessageRef, Numeric, ParseError, Source};
use proptest::{collection::vec, option, prelude::*};
use serde_json::Value;

fn valid_message() -> impl Strategy<Value = Message> {
    let tag_value = vec(
        any::<char>().prop_filter("NUL cannot be serialized", |character| *character != '\0'),
        0..16,
    )
    .prop_map(|characters| characters.into_iter().collect::<String>());

    (
        "[A-Z]{1,10}",
        option::of("[!-~]{1,24}"),
        vec((r"\+?[A-Za-z0-9./_-]{1,12}", tag_value), 0..4),
        vec("[-A-Za-z0-9#._/+~]{1,16}", 0..4),
        option::of("[ -~]{0,24}"),
    )
        .prop_map(|(command, source, tags, mut middle, trailing)| {
            middle.extend(trailing);
            let mut message = Message::new(command, middle);
            if let Some(source) = source {
                message = message.with_source(source);
            }
            for (key, value) in tags {
                message = message.with_tag(key, value);
            }
            message
        })
}

proptest! {
    #[test]
    fn owned_messages_round_trip(message in valid_message()) {
        let line = message.to_line().unwrap();
        let reparsed = MessageRef::parse(&line).unwrap().to_owned();
        prop_assert_eq!(reparsed, message);
    }
}

#[test]
fn passes_ircdocs_message_split_corpus() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-split.json")).unwrap();

    for case in corpus["tests"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let expected = &case["atoms"];
        let message = MessageRef::parse(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));

        assert_eq!(
            message.raw_source(),
            expected["source"].as_str(),
            "{input:?}"
        );
        assert_eq!(
            message.raw_command(),
            expected["verb"].as_str().unwrap(),
            "{input:?}"
        );

        let actual_params: Vec<_> = message.params().collect();
        let expected_params: Vec<_> = expected["params"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|param| param.as_str().unwrap())
            .collect();
        assert_eq!(actual_params, expected_params, "{input:?}");

        let actual_tags: BTreeMap<_, _> = message
            .tags()
            .map(|(key, value)| (key, value.unwrap_or_default().into_owned()))
            .collect();
        let expected_tags: BTreeMap<_, _> = expected["tags"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str().unwrap().to_owned()))
            .collect();
        assert_eq!(actual_tags, expected_tags, "{input:?}");
    }
}

#[test]
fn passes_ircdocs_userhost_split_corpus() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/userhost-split.json")).unwrap();

    for case in corpus["tests"].as_array().unwrap() {
        let raw = case["source"].as_str().unwrap();
        let expected = &case["atoms"];
        let line = format!(":{raw} NOTICE papo-test :maintenance");
        let message = MessageRef::parse(&line).unwrap();

        assert_eq!(
            message.source(),
            Some(Source::User {
                nick: expected["nick"].as_str().unwrap(),
                user: expected["user"].as_str(),
                host: expected["host"].as_str(),
            }),
            "{raw:?}"
        );
    }
}

#[test]
fn parses_ergo_user_sources_and_server_names() {
    let cases = [
        (
            "nick!~u@abc123.irc",
            Source::User {
                nick: "nick",
                user: Some("~u"),
                host: Some("abc123.irc"),
            },
        ),
        (
            "*name*!realnick@npc.fakeuser.invalid",
            Source::User {
                nick: "*name*",
                user: Some("realnick"),
                host: Some("npc.fakeuser.invalid"),
            },
        ),
        (
            "nick/x!~u@cloak",
            Source::User {
                nick: "nick/x",
                user: Some("~u"),
                host: Some("cloak"),
            },
        ),
        ("irc.example.net", Source::Server("irc.example.net")),
    ];

    for (raw, expected) in cases {
        let line = format!(":{raw} PRIVMSG #papo :hello");
        let message = MessageRef::parse(&line).unwrap();
        assert_eq!(message.raw_source(), Some(raw));
        assert_eq!(message.source(), Some(expected));
    }
}

#[test]
fn normalizes_empty_user_source_parts() {
    let cases = [
        (
            "nick!",
            Source::User {
                nick: "nick",
                user: None,
                host: None,
            },
        ),
        (
            "nick!@host",
            Source::User {
                nick: "nick",
                user: None,
                host: Some("host"),
            },
        ),
        (
            "nick@",
            Source::User {
                nick: "nick",
                user: None,
                host: None,
            },
        ),
    ];

    for (raw, expected) in cases {
        let line = format!(":{raw} NOTICE papo-test :maintenance");
        assert_eq!(MessageRef::parse(&line).unwrap().source(), Some(expected));
    }
}

#[test]
fn source_free_ircdocs_lines_round_trip_through_serialization() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-split.json")).unwrap();
    let mut tested = 0;

    for case in corpus["tests"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let parsed = MessageRef::parse(input).unwrap();
        if parsed.raw_source().is_some() {
            continue;
        }
        let message = parsed.to_owned();
        let line = message.to_line().unwrap();
        assert_eq!(
            MessageRef::parse(&line).unwrap().to_owned(),
            message,
            "{input:?}"
        );
        tested += 1;
    }

    assert_eq!(tested, 14);
}

#[test]
fn parses_realistic_ergo_lines() {
    let message = MessageRef::parse(
        "@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123 :alice!~u@example.test PRIVMSG #papo :hello there\r\n",
    )
    .unwrap();
    assert_eq!(message.raw_source(), Some("alice!~u@example.test"));
    assert_eq!(message.command(), Command::Privmsg);
    assert!(message.command_is("privmsg"));
    assert!(!message.command_is("NOTICE"));
    assert_eq!(message.tag("msgid"), Some(Cow::Borrowed("abcDEF123")));
    assert_eq!(
        message.params().collect::<Vec<_>>(),
        ["#papo", "hello there"]
    );

    let batch = MessageRef::parse(":ergo.test BATCH +history chathistory #papo").unwrap();
    assert_eq!(
        batch.params().collect::<Vec<_>>(),
        ["+history", "chathistory", "#papo"]
    );

    let cap = MessageRef::parse(":ergo.test CAP * LS * :account-notify batch server-time").unwrap();
    assert_eq!(
        cap.params().collect::<Vec<_>>(),
        ["*", "LS", "*", "account-notify batch server-time"]
    );

    let authenticate = MessageRef::parse("AUTHENTICATE +\n").unwrap();
    assert_eq!(authenticate.raw_source(), None);
    assert_eq!(authenticate.param(0), Some("+"));

    let ping = MessageRef::parse("PING").unwrap();
    assert!(ping.is_empty());

    let forced_trailing = MessageRef::parse(":ergo.test 323 papo-test :").unwrap();
    assert_eq!(forced_trailing.command(), Command::Numeric(Numeric(323)));
    assert_eq!(forced_trailing.param(1), Some(""));
}

#[test]
fn parses_literal_wire_commands() {
    let cases = [
        ("NICK papo-test", Command::Nick),
        ("USER papo 0 * :papo test", Command::User),
        ("PING :irc.example.net", Command::Ping),
        ("PRIVMSG #papo :hello", Command::Privmsg),
        ("JOIN #papo", Command::Join),
        ("CAP LS 302", Command::Cap),
        ("AUTHENTICATE PLAIN", Command::Authenticate),
        ("BATCH +history chathistory #papo", Command::Batch),
        ("CHATHISTORY LATEST #papo * 50", Command::Chathistory),
        (
            "MARKREAD #papo timestamp=2026-10-14T20:15:42.123Z",
            Command::Markread,
        ),
        ("RELAYMSG #papo nick/relay :hello", Command::Relaymsg),
        ("EXTJWT * papo", Command::Extjwt),
        (
            ":ergo.test FAIL * ACCOUNT_REQUIRED :Authentication required",
            Command::Fail,
        ),
        ("METADATA * GET display-name", Command::Metadata),
        (
            "REGISTER papo-test * :correct horse battery staple",
            Command::Register,
        ),
    ];

    for (line, expected) in cases {
        assert_eq!(
            MessageRef::parse(line).unwrap().command(),
            expected,
            "{line:?}"
        );
    }
}

#[test]
fn parses_literal_wire_numeric_codes_and_names() {
    let cases = [
        (
            ":ergo.test 001 papo-test :Welcome",
            1,
            "RPL_WELCOME",
            Numeric::RPL_WELCOME,
        ),
        (
            ":ergo.test 005 papo-test MONITOR=100 :are supported",
            5,
            "RPL_ISUPPORT",
            Numeric::RPL_ISUPPORT,
        ),
        (
            ":ergo.test 335 papo-test bot :is a bot",
            335,
            "RPL_WHOISBOT",
            Numeric::RPL_WHOISBOT,
        ),
        (
            ":ergo.test 354 papo-test 42 #papo ~u host irc.example.net nick H 0 :real name",
            354,
            "RPL_WHOSPCRPL",
            Numeric::RPL_WHOSPCRPL,
        ),
        (
            ":ergo.test 396 papo-test cloak.example :is now your displayed host",
            396,
            "RPL_VISIBLEHOST",
            Numeric::RPL_VISIBLEHOST,
        ),
        (
            ":ergo.test 417 papo-test :Input line was too long",
            417,
            "ERR_INPUTTOOLONG",
            Numeric::ERR_INPUTTOOLONG,
        ),
        (
            ":ergo.test 470 papo-test #old #new :Forwarding",
            470,
            "ERR_LINKCHANNEL",
            Numeric::ERR_LINKCHANNEL,
        ),
        (
            ":ergo.test 730 papo-test :nick!~u@cloak",
            730,
            "RPL_MONONLINE",
            Numeric::RPL_MONONLINE,
        ),
        (
            ":ergo.test 760 papo-test nick display-name * :Papo Bot",
            760,
            "RPL_WHOISKEYVALUE",
            Numeric::RPL_WHOISKEYVALUE,
        ),
        (
            ":ergo.test 903 papo-test :SASL authentication successful",
            903,
            "RPL_SASLSUCCESS",
            Numeric::RPL_SASLSUCCESS,
        ),
        (
            ":ergo.test 904 papo-test :SASL authentication failed",
            904,
            "ERR_SASLFAIL",
            Numeric::ERR_SASLFAIL,
        ),
    ];

    for (line, code, name, expected) in cases {
        let Command::Numeric(numeric) = MessageRef::parse(line).unwrap().command() else {
            panic!("{line:?} did not parse as a numeric");
        };
        assert_eq!(numeric, expected, "{line:?}");
        assert_eq!(numeric.0, code, "{line:?}");
        assert_eq!(numeric.name(), Some(name), "{line:?}");
    }
}

#[test]
fn parses_tags_and_source_with_irregular_spacing() {
    let message = MessageRef::parse(
        "@time=2026-10-14T20:15:42.123Z   :alice!~u@example.test   PRIVMSG #papo :hello",
    )
    .unwrap();

    assert_eq!(
        message.tag("time"),
        Some(Cow::Borrowed("2026-10-14T20:15:42.123Z"))
    );
    assert_eq!(message.raw_source(), Some("alice!~u@example.test"));
    assert_eq!(message.command(), Command::Privmsg);
}

#[test]
fn unescapes_tags_and_uses_the_last_duplicate() {
    let message = MessageRef::parse(
        r"@draft/example=semi\:colon\sand\\slash\r\nknown\q;account=;batch;batch=42;Case=UP PRIVMSG #papo :tagged",
    )
    .unwrap();

    assert_eq!(
        message.tag("draft/example"),
        Some(Cow::Owned("semi;colon and\\slash\r\nknownq".to_owned()))
    );
    assert_eq!(message.tag("account"), None);
    assert_eq!(message.tag("batch"), Some(Cow::Borrowed("42")));
    assert_eq!(message.tag("case"), None);
    assert_eq!(message.tag("Case"), Some(Cow::Borrowed("UP")));

    let trailing_slash = MessageRef::parse(r"@example=value\ PRIVMSG #papo :text").unwrap();
    assert_eq!(
        trailing_slash.tag("example"),
        Some(Cow::Owned("value".to_owned()))
    );
}

#[test]
fn distinguishes_empty_lines_from_missing_commands() {
    assert_eq!(MessageRef::parse(""), Err(ParseError::EmptyLine));
    assert_eq!(MessageRef::parse("\r\n"), Err(ParseError::EmptyLine));
    assert_eq!(MessageRef::parse("   "), Err(ParseError::MissingCommand));
    assert_eq!(
        MessageRef::parse("@time=2026-10-14T20:15:42.123Z"),
        Err(ParseError::MissingCommand)
    );
    assert_eq!(
        MessageRef::parse(":ergo.test"),
        Err(ParseError::MissingCommand)
    );
}

#[test]
fn parses_many_parameters_with_irregular_spacing() {
    let input = format!(
        "COMMAND {}",
        (1..=20)
            .map(|number| number.to_string())
            .collect::<Vec<_>>()
            .join("  ")
    );
    let message = MessageRef::parse(&input).unwrap();

    assert_eq!(message.len(), 20);
    assert!(!message.is_empty());
    assert_eq!(message.param(0), Some("1"));
    assert_eq!(message.param(14), Some("15"));
    assert_eq!(message.param(19), Some("20"));
    assert_eq!(message.param(20), None);
}

#[test]
fn rejects_non_utf8_bytes() {
    assert_eq!(
        MessageRef::parse_bytes(b"PRIVMSG #papo :invalid \xff"),
        Err(ParseError::InvalidUtf8)
    );
}

#[test]
fn displays_each_parse_error() {
    assert_eq!(ParseError::EmptyLine.to_string(), "IRC line is empty");
    assert_eq!(
        ParseError::MissingCommand.to_string(),
        "IRC line has no command"
    );
    assert_eq!(
        ParseError::InvalidUtf8.to_string(),
        "IRC line is not valid UTF-8"
    );
    assert_eq!(
        ParseError::TooLong.to_string(),
        "IRC line exceeds its wire length limit"
    );
}

#[test]
fn enforces_tag_section_length_at_the_byte_boundary() {
    let maximum = format!("@{} PING", "a".repeat(8191 - 2));
    assert!(MessageRef::parse(&maximum).is_ok());

    let over_limit = format!("@{} PING", "a".repeat(8191 - 1));
    assert_eq!(MessageRef::parse(&over_limit), Err(ParseError::TooLong));
}

#[test]
fn enforces_message_length_at_the_byte_boundary() {
    let prefix = "PRIVMSG #papo :";
    let maximum_body = format!("{prefix}{}", "x".repeat(512 - 2 - prefix.len()));
    let maximum = format!("@time=2026-10-14T20:15:42.123Z {maximum_body}\r\n");
    assert!(MessageRef::parse(&maximum).is_ok());

    let over_limit = format!("{maximum_body}x\r\n");
    assert_eq!(MessageRef::parse(&over_limit), Err(ParseError::TooLong));
}

#[test]
fn counts_canonical_crlf_for_a_bare_lf_line() {
    let prefix = "NOTICE papo-test :";
    let line = format!("{prefix}{}\n", "x".repeat(512 - 2 - prefix.len()));

    assert_eq!(line.len() + 1, 512);
    assert!(MessageRef::parse(&line).is_ok());
}

#[test]
fn enforces_both_limits_after_leading_spaces() {
    let maximum_tags = format!("  @{} PING", "a".repeat(8191 - 2));
    assert!(MessageRef::parse(&maximum_tags).is_ok());
    let excessive_tags = format!("  @{} PING", "a".repeat(8191 - 1));
    assert_eq!(MessageRef::parse(&excessive_tags), Err(ParseError::TooLong));

    let prefix = "PRIVMSG #papo :";
    let maximum_body = format!("{prefix}{}", "x".repeat(512 - 2 - prefix.len()));
    let maximum_message = format!("  @time=2026-10-14T20:15:42.123Z {maximum_body}\r\n");
    assert!(MessageRef::parse(&maximum_message).is_ok());
    let excessive_message = format!("  @time=2026-10-14T20:15:42.123Z {maximum_body}x\r\n");
    assert_eq!(
        MessageRef::parse(&excessive_message),
        Err(ParseError::TooLong)
    );
}

#[test]
fn owns_parsed_messages_without_reparsing() {
    let parsed = MessageRef::parse(
        r"@+draft/reply=old;+typing=active;+draft/reply=new\svalue :papo-test!~papo@example.test PRIVMSG #papo :hello there",
    )
    .unwrap();
    let message = parsed.to_owned();
    let borrowed = message.as_ref();

    assert_eq!(borrowed.raw_source(), Some("papo-test!~papo@example.test"));
    assert_eq!(borrowed.command(), Command::Privmsg);
    assert_eq!(
        borrowed.params().collect::<Vec<_>>(),
        ["#papo", "hello there"]
    );
    assert_eq!(
        borrowed.tags().collect::<Vec<_>>(),
        [
            ("+draft/reply", Some(Cow::Borrowed("new value"))),
            ("+typing", Some(Cow::Borrowed("active"))),
        ]
    );
}

#[test]
fn builds_common_client_commands() {
    let tagged = Message::privmsg("#papo", "hello there")
        .with_tag("+draft/reply", "abcDEF123")
        .with_tag("+typing", "active")
        .with_tag("+draft/reply", "new-id")
        .with_source("papo-test!~papo@example.test");
    assert_eq!(
        tagged.as_ref().tags().collect::<Vec<_>>(),
        [
            ("+draft/reply", Some(Cow::Borrowed("new-id"))),
            ("+typing", Some(Cow::Borrowed("active"))),
        ]
    );

    let cases = [
        (
            Message::notice("papo-test", "maintenance soon"),
            "NOTICE",
            2,
        ),
        (Message::tagmsg("#papo"), "TAGMSG", 1),
        (Message::join("#papo"), "JOIN", 1),
        (Message::part("#papo"), "PART", 1),
        (Message::part_with_reason("#papo", "leaving now"), "PART", 2),
        (Message::new("CAP", ["REQ", "message-tags"]), "CAP", 2),
    ];
    for (message, command, param_count) in cases {
        assert_eq!(message.as_ref().raw_command(), command);
        assert_eq!(message.as_ref().len(), param_count);
    }
}

#[test]
fn passes_ircdocs_message_join_corpus() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-join.json")).unwrap();

    for case in corpus["tests"].as_array().unwrap() {
        let matches = case["matches"].as_array().unwrap();
        let input = matches[0].as_str().unwrap();
        let output = MessageRef::parse(input)
            .unwrap()
            .to_owned()
            .to_line()
            .unwrap();
        assert!(
            matches
                .iter()
                .any(|candidate| candidate.as_str() == output.strip_suffix("\r\n")),
            "{}: {output:?}",
            case["desc"]
        );
    }
}

#[test]
fn serializes_realistic_client_lines() {
    let reply = Message::privmsg("#papo", "hello there")
        .with_tag("+draft/reply", "abcDEF123")
        .with_tag("+reply", "xyz789");
    assert_eq!(
        reply.to_line().unwrap(),
        "@+draft/reply=abcDEF123;+reply=xyz789 PRIVMSG #papo :hello there\r\n"
    );

    let chunk = "dGVzdABwYXBvLXRlc3QAc2VjcmV0".repeat(12);
    assert_eq!(
        Message::new("AUTHENTICATE", [&chunk]).to_line().unwrap(),
        format!("AUTHENTICATE {chunk}\r\n")
    );
    assert_eq!(
        Message::new(
            "CAP",
            ["REQ", "message-tags server-time batch labeled-response"]
        )
        .to_line()
        .unwrap(),
        "CAP REQ :message-tags server-time batch labeled-response\r\n"
    );
}

#[test]
fn rejects_forbidden_command_and_parameter_bytes() {
    for command in ["", "PRIV MSG", "PRIV\rMSG", "PRIV\nMSG", "PRIV\0MSG"] {
        assert_eq!(
            Message::new(command, ["#papo", "hello"]).to_line(),
            Err(BuildError::InvalidCommand)
        );
    }
    for param in ["hello\rthere", "hello\nthere", "hello\0there"] {
        assert_eq!(
            Message::privmsg("#papo", param).to_line(),
            Err(BuildError::InvalidParam(1))
        );
    }
}

#[test]
fn rejects_fields_that_could_change_wire_structure() {
    for source in [
        "",
        "bad source",
        "bad\rsource",
        "bad\nsource",
        "bad\0source",
    ] {
        assert_eq!(
            Message::new("NOTICE", ["papo-test", "maintenance"])
                .with_source(source)
                .to_line(),
            Err(BuildError::InvalidSource)
        );
    }
    for key in [
        "", "bad key", "bad;key", "bad=key", "bad\rkey", "bad\nkey", "bad\0key",
    ] {
        assert_eq!(
            Message::tagmsg("#papo").with_tag(key, "active").to_line(),
            Err(BuildError::InvalidTagKey)
        );
    }
    assert_eq!(
        Message::tagmsg("#papo")
            .with_tag("+example.test/status", "active\0idle")
            .to_line(),
        Err(BuildError::InvalidTagValue)
    );
    assert_eq!(
        Message::tagmsg("#papo")
            .with_tag("+typing", "")
            .to_line()
            .unwrap(),
        "@+typing TAGMSG #papo\r\n"
    );
}

#[test]
fn requires_middle_parameters_to_use_middle_form() {
    for invalid in ["", "two words", ":prefixed"] {
        assert_eq!(
            Message::new("COMMAND", [invalid, "last"]).to_line(),
            Err(BuildError::InvalidParam(0))
        );
    }

    assert_eq!(
        Message::new("COMMAND", [""]).to_line().unwrap(),
        "COMMAND :\r\n"
    );
    assert_eq!(
        Message::new("COMMAND", ["two words"]).to_line().unwrap(),
        "COMMAND :two words\r\n"
    );
    assert_eq!(
        Message::new("COMMAND", [":prefixed"]).to_line().unwrap(),
        "COMMAND ::prefixed\r\n"
    );
    assert_eq!(
        Message::new("COMMAND", ["ordinary"]).to_line().unwrap(),
        "COMMAND ordinary\r\n"
    );
}

#[test]
fn enforces_client_tag_data_length_at_the_byte_boundary() {
    let prefix_len = "+example.test/reply=".len();
    let maximum =
        Message::tagmsg("#papo").with_tag("+example.test/reply", "x".repeat(4094 - prefix_len));
    assert!(maximum.to_line().is_ok());

    let over_limit =
        Message::tagmsg("#papo").with_tag("+example.test/reply", "x".repeat(4094 - prefix_len + 1));
    assert_eq!(over_limit.to_line(), Err(BuildError::TagsTooLong));
}

#[test]
fn enforces_serialized_body_length_at_the_byte_boundary() {
    let prefix = "PRIVMSG #papo ";
    let maximum =
        Message::privmsg("#papo", "x".repeat(512 - 2 - prefix.len())).with_tag("+typing", "active");
    let maximum_line = maximum.to_line().unwrap();
    assert_eq!(maximum_line.split_once(' ').unwrap().1.len(), 512);

    let over_limit = Message::privmsg("#papo", "x".repeat(512 - 2 - prefix.len() + 1))
        .with_tag("+typing", "active");
    assert_eq!(over_limit.to_line(), Err(BuildError::MessageTooLong));
}

#[test]
fn displays_each_build_error() {
    assert_eq!(
        BuildError::InvalidCommand.to_string(),
        "IRC command is invalid"
    );
    assert_eq!(
        BuildError::InvalidParam(2).to_string(),
        "IRC parameter 2 is invalid"
    );
    assert_eq!(
        BuildError::InvalidSource.to_string(),
        "IRC source is invalid"
    );
    assert_eq!(
        BuildError::InvalidTagKey.to_string(),
        "IRC tag key is invalid"
    );
    assert_eq!(
        BuildError::InvalidTagValue.to_string(),
        "IRC tag value is invalid"
    );
    assert_eq!(
        BuildError::TagsTooLong.to_string(),
        "IRC client tag data exceeds its wire limit"
    );
    assert_eq!(
        BuildError::MessageTooLong.to_string(),
        "IRC message body exceeds its wire limit"
    );
}
