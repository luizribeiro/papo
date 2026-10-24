//! Integration tests for borrowed IRC line parsing.

use std::{borrow::Cow, collections::BTreeMap};

use papo_proto::{BuildError, Command, Line, LineRef, Numeric, ParseError, Secret, Source};
use proptest::{collection::vec, option, prelude::*};
use serde_json::Value;

fn valid_line() -> impl Strategy<Value = Line> {
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
            let mut line = Line::new(command).with_params(middle);
            if let Some(source) = source {
                line = line.with_source(source);
            }
            for (key, value) in tags {
                line = line.with_tag(key, value);
            }
            line
        })
}

proptest! {
    #[test]
    fn owned_lines_round_trip(line in valid_line()) {
        let serialized = line.to_line().unwrap();
        let reparsed = LineRef::parse(&serialized).unwrap().to_owned();
        prop_assert_eq!(reparsed, line);
    }
}

#[test]
fn passes_ircdocs_line_split_corpus() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-split.json")).unwrap();

    for case in corpus["tests"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let expected = &case["atoms"];
        let line = LineRef::parse(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));

        assert_eq!(line.raw_source(), expected["source"].as_str(), "{input:?}");
        assert_eq!(
            line.raw_command(),
            expected["verb"].as_str().unwrap(),
            "{input:?}"
        );

        let actual_params: Vec<_> = line.params().collect();
        let expected_params: Vec<_> = expected["params"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|param| param.as_str().unwrap())
            .collect();
        assert_eq!(actual_params, expected_params, "{input:?}");

        let actual_tags: BTreeMap<_, _> = line
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
        let line = LineRef::parse(&line).unwrap();

        assert_eq!(
            line.source(),
            Some(Source::__user(
                expected["nick"].as_str().unwrap(),
                expected["user"].as_str(),
                expected["host"].as_str(),
            )),
            "{raw:?}"
        );
    }
}

#[test]
fn parses_ergo_user_sources_and_server_names() {
    let cases = [
        (
            "nick!~u@abc123.irc",
            Source::__user("nick", Some("~u"), Some("abc123.irc")),
        ),
        (
            "*name*!realnick@npc.fakeuser.invalid",
            Source::__user("*name*", Some("realnick"), Some("npc.fakeuser.invalid")),
        ),
        (
            "nick/x!~u@cloak",
            Source::__user("nick/x", Some("~u"), Some("cloak")),
        ),
        ("irc.example.net", Source::Server("irc.example.net")),
    ];

    for (raw, expected) in cases {
        let line = format!(":{raw} PRIVMSG #papo :hello");
        let line = LineRef::parse(&line).unwrap();
        assert_eq!(line.raw_source(), Some(raw));
        assert_eq!(line.source(), Some(expected));
    }
}

#[test]
fn normalizes_empty_user_source_parts() {
    let cases = [
        ("nick!", Source::__user("nick", None, None)),
        ("nick!@host", Source::__user("nick", None, Some("host"))),
        ("nick@", Source::__user("nick", None, None)),
    ];

    for (raw, expected) in cases {
        let line = format!(":{raw} NOTICE papo-test :maintenance");
        assert_eq!(LineRef::parse(&line).unwrap().source(), Some(expected));
    }
}

#[test]
fn source_free_ircdocs_lines_round_trip_through_serialization() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-split.json")).unwrap();
    let mut tested = 0;

    for case in corpus["tests"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let parsed = LineRef::parse(input).unwrap();
        if parsed.raw_source().is_some() {
            continue;
        }
        let line = parsed.to_owned();
        let serialized = line.to_line().unwrap();
        assert_eq!(
            LineRef::parse(&serialized).unwrap().to_owned(),
            line,
            "{input:?}"
        );
        tested += 1;
    }

    assert_eq!(tested, 14);
}

#[test]
fn parses_realistic_ergo_lines() {
    let line = LineRef::parse(
        "@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123 :alice!~u@example.test PRIVMSG #papo :hello there\r\n",
    )
    .unwrap();
    assert_eq!(line.raw_source(), Some("alice!~u@example.test"));
    assert_eq!(line.command(), Command::Privmsg);
    assert!(line.command_is("privmsg"));
    assert!(!line.command_is("NOTICE"));
    assert!(matches!(
        line.tag("msgid"),
        Some(Cow::Borrowed("abcDEF123"))
    ));
    assert_eq!(line.params().collect::<Vec<_>>(), ["#papo", "hello there"]);

    let batch = LineRef::parse(":ergo.test BATCH +history chathistory #papo").unwrap();
    assert_eq!(
        batch.params().collect::<Vec<_>>(),
        ["+history", "chathistory", "#papo"]
    );

    let cap = LineRef::parse(":ergo.test CAP * LS * :account-notify batch server-time").unwrap();
    assert_eq!(
        cap.params().collect::<Vec<_>>(),
        ["*", "LS", "*", "account-notify batch server-time"]
    );

    let authenticate = LineRef::parse("AUTHENTICATE +\n").unwrap();
    assert_eq!(authenticate.raw_source(), None);
    assert_eq!(authenticate.get(0), Some("+"));

    let ping = LineRef::parse("PING").unwrap();
    assert!(ping.is_empty());

    let forced_trailing = LineRef::parse(":ergo.test 323 papo-test :").unwrap();
    assert_eq!(
        forced_trailing.command(),
        Command::Numeric(Numeric::RPL_LISTEND)
    );
    assert_eq!(forced_trailing.get(1), Some(""));
}

#[test]
fn owned_lines_parse_real_ergo_messages() {
    let message: Line = "@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123 :alice!~a@cloak.example PRIVMSG #papo :release ready"
        .parse()
        .unwrap();
    let borrowed = message.as_ref();
    assert_eq!(borrowed.raw_command(), "PRIVMSG");
    assert_eq!(
        borrowed.params().collect::<Vec<_>>(),
        ["#papo", "release ready"]
    );
    assert_eq!(
        borrowed.tag("time").as_deref(),
        Some("2026-10-14T20:15:42.123Z")
    );
    assert_eq!(borrowed.tag("msgid").as_deref(), Some("abcDEF123"));
    assert!(matches!(
        borrowed.source(),
        Some(Source::User {
            nick: "alice",
            username: Some("~a"),
            host: Some("cloak.example"),
            ..
        })
    ));

    let numeric: Line = ":ergo.test 372 papo :- Welcome to ErgoTest"
        .parse()
        .unwrap();
    assert_eq!(
        numeric.as_ref().command(),
        Command::Numeric(Numeric::new(372).unwrap())
    );
    assert_eq!(numeric.as_ref().get(1), Some("- Welcome to ErgoTest"));
    assert_eq!(
        numeric.to_string(),
        ":ergo.test 372 papo :- Welcome to ErgoTest"
    );
}

#[test]
fn reencodes_realistic_server_lines_with_exact_byte_totals() {
    let cases = [
        (
            "@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123 :alice!~a@cloak.example PRIVMSG #papo :release ready",
            101,
        ),
        (
            ":solanum.example 728 papo #papo q *!*@quiet.example oper 1761048000",
            69,
        ),
        (
            r"@batch=networks :soju.example BOUNCER NETWORK 42 name=My\sNetwork;state=connected;port=6697;tls=1",
            99,
        ),
    ];

    for (server_line, wire_len) in cases {
        let encoded = server_line.parse::<Line>().unwrap().to_line().unwrap();
        assert_eq!(encoded, format!("{server_line}\r\n"));
        assert_eq!(encoded.len(), wire_len);
        assert_eq!(encoded.capacity(), wire_len);
    }
}

#[test]
fn owned_line_parsing_rejects_delimiters_and_empty_input() {
    for input in [
        "PRIVMSG #papo :hello\rthere",
        "PRIVMSG #papo :hello\nthere",
        "PRIVMSG #papo :hello\0there",
    ] {
        assert_eq!(input.parse::<Line>(), Err(ParseError::InvalidByte));
    }
    assert_eq!("".parse::<Line>(), Err(ParseError::EmptyLine));
}

#[test]
fn owned_lines_format_parameters_and_reject_invalid_content() {
    assert_eq!(
        Line::new("PRIVMSG")
            .param("#papo")
            .param("release ready")
            .to_string(),
        "PRIVMSG #papo :release ready"
    );
    assert_eq!(
        Line::new("KILL")
            .param("alice evil")
            .param("policy violation")
            .to_line(),
        Err(BuildError::InvalidParam(0))
    );
    assert_eq!(
        Line::new("NOTICE")
            .param("papo")
            .param("bad\nQUIT")
            .to_line(),
        Err(BuildError::InvalidParam(1))
    );
}

#[test]
fn owned_line_encoding_allocates_its_exact_wire_length() {
    let line = Line::new("PRIVMSG")
        .param("#papo")
        .param("release ready")
        .with_tag("+draft/reply", "RfcBTzyxY120004");
    let encoded = line.to_line().unwrap();
    assert_eq!(
        encoded,
        "@+draft/reply=RfcBTzyxY120004 PRIVMSG #papo :release ready\r\n"
    );
    assert_eq!(encoded.capacity(), encoded.len());
}

#[test]
fn owned_lines_expose_every_wire_component() {
    let line: Line = "@bot;account=old;account=alice :ergo.test 372 papo :- Welcome to ErgoTest"
        .parse()
        .unwrap();

    assert_eq!(line.command(), "372");
    assert_eq!(line.numeric(), Some(372));
    assert_eq!(line.params(), ["papo", "- Welcome to ErgoTest"]);
    assert_eq!(line.get(0), Some("papo"));
    assert_eq!(line.get(2), None);
    assert_eq!(line.tag("bot"), Some(""));
    assert_eq!(line.tag("account"), Some("alice"));
    assert_eq!(
        line.tags().collect::<Vec<_>>(),
        [("bot", ""), ("account", "old"), ("account", "alice")]
    );
    assert_eq!(line.source(), Some(Source::Server("ergo.test")));
    assert_eq!(
        line.to_string(),
        "@bot;account=old;account=alice :ergo.test 372 papo :- Welcome to ErgoTest"
    );
}

#[test]
fn owned_line_numeric_rejects_non_numeric_and_out_of_range_commands() {
    assert_eq!(Line::new("WALLOPS").param("maintenance").numeric(), None);
    assert_eq!(Line::new("1000").param("not numeric").numeric(), None);
    assert_eq!(Line::new("000").param("numeric zero").numeric(), Some(0));
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
            LineRef::parse(line).unwrap().command(),
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
        let Command::Numeric(numeric) = LineRef::parse(line).unwrap().command() else {
            panic!("{line:?} did not parse as a numeric");
        };
        assert_eq!(numeric, expected, "{line:?}");
        assert_eq!(numeric.code(), code, "{line:?}");
        assert_eq!(numeric.name(), Some(name), "{line:?}");
    }
}

#[test]
fn parses_tags_and_source_with_irregular_spacing() {
    let line = LineRef::parse(
        "@time=2026-10-14T20:15:42.123Z   :alice!~u@example.test   PRIVMSG #papo :hello",
    )
    .unwrap();

    assert!(matches!(
        line.tag("time"),
        Some(Cow::Borrowed("2026-10-14T20:15:42.123Z"))
    ));
    assert_eq!(line.raw_source(), Some("alice!~u@example.test"));
    assert_eq!(line.command(), Command::Privmsg);
}

#[test]
fn unescapes_tags_and_uses_the_last_duplicate() {
    let line = LineRef::parse(
        r"@draft/example=semi\:colon\sand\\slash\r\nknown\q;account=;batch;batch=42;Case=UP PRIVMSG #papo :tagged",
    )
    .unwrap();

    assert!(matches!(
        line.tag("draft/example"),
        Some(Cow::Owned(value)) if value == "semi;colon and\\slash\r\nknownq"
    ));
    assert_eq!(line.tag("account"), None);
    assert!(matches!(line.tag("batch"), Some(Cow::Borrowed("42"))));
    assert_eq!(line.tag("case"), None);
    assert!(matches!(line.tag("Case"), Some(Cow::Borrowed("UP"))));

    let trailing_slash = LineRef::parse(r"@example=value\ PRIVMSG #papo :text").unwrap();
    assert!(matches!(
        trailing_slash.tag("example"),
        Some(Cow::Owned(value)) if value == "value"
    ));
}

#[test]
fn normalizes_empty_and_valueless_tags() {
    let empty = LineRef::parse("@+example.com/foo=;msgid=abc :nick!u@h PRIVMSG #c :hi").unwrap();
    assert!(matches!(
        empty.tags().collect::<Vec<_>>().as_slice(),
        [
            ("+example.com/foo", None),
            ("msgid", Some(Cow::Borrowed("abc"))),
        ]
    ));
    assert_eq!(
        empty.to_owned().to_string(),
        "@+example.com/foo;msgid=abc :nick!u@h PRIVMSG #c hi"
    );

    let valueless = LineRef::parse("@foo :srv 001 me :hi").unwrap();
    assert!(matches!(
        valueless.tags().collect::<Vec<_>>().as_slice(),
        [("foo", None)]
    ));
    assert_eq!(valueless.to_owned().to_string(), "@foo :srv 001 me hi");

    let built = Line::tagmsg("#c").with_tag("+example.com/foo", "");
    assert!(matches!(
        built.as_ref().tags().collect::<Vec<_>>().as_slice(),
        [("+example.com/foo", None)]
    ));
}

#[test]
fn minimized_tag_round_trip_preserves_unknown_command() {
    let input = "@ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ=\\ +";
    let owned = LineRef::parse(input).unwrap().to_owned();

    assert_eq!(owned.command(), "+");
    assert!(matches!(
        owned.as_ref().tags().collect::<Vec<_>>().as_slice(),
        [("ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ", None)]
    ));
    assert_eq!(owned.to_string(), "@ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ +");
    assert_eq!(
        LineRef::parse(&owned.to_string()).unwrap().to_owned(),
        owned
    );
}

#[test]
fn distinguishes_empty_lines_from_missing_commands() {
    assert_eq!(LineRef::parse(""), Err(ParseError::EmptyLine));
    assert_eq!(LineRef::parse("\r\n"), Err(ParseError::EmptyLine));
    assert_eq!(LineRef::parse("   "), Err(ParseError::MissingCommand));
    assert_eq!(
        LineRef::parse("@time=2026-10-14T20:15:42.123Z"),
        Err(ParseError::MissingCommand)
    );
    assert_eq!(
        LineRef::parse(":ergo.test"),
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
    let line = LineRef::parse(&input).unwrap();

    assert_eq!(line.len(), 20);
    assert!(!line.is_empty());
    assert_eq!(line.get(0), Some("1"));
    assert_eq!(line.get(14), Some("15"));
    assert_eq!(line.get(19), Some("20"));
    assert_eq!(line.get(20), None);
}

#[test]
fn rejects_non_utf8_bytes() {
    assert_eq!(
        LineRef::parse_bytes(b"PRIVMSG #papo :invalid \xff"),
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
    assert!(LineRef::parse(&maximum).is_ok());

    let over_limit = format!("@{} PING", "a".repeat(8191 - 1));
    assert_eq!(LineRef::parse(&over_limit), Err(ParseError::TooLong));
}

#[test]
fn enforces_line_length_at_the_byte_boundary() {
    let prefix = "PRIVMSG #papo :";
    let maximum_body = format!("{prefix}{}", "x".repeat(512 - 2 - prefix.len()));
    let maximum = format!("@time=2026-10-14T20:15:42.123Z {maximum_body}\r\n");
    assert!(LineRef::parse(&maximum).is_ok());

    let over_limit = format!("{maximum_body}x\r\n");
    assert_eq!(LineRef::parse(&over_limit), Err(ParseError::TooLong));
}

#[test]
fn counts_canonical_crlf_for_a_bare_lf_line() {
    let prefix = "NOTICE papo-test :";
    let line = format!("{prefix}{}\n", "x".repeat(512 - 2 - prefix.len()));

    assert_eq!(line.len() + 1, 512);
    assert!(LineRef::parse(&line).is_ok());
}

#[test]
fn enforces_both_limits_after_leading_spaces() {
    let maximum_tags = format!("  @{} PING", "a".repeat(8191 - 2));
    assert!(LineRef::parse(&maximum_tags).is_ok());
    let excessive_tags = format!("  @{} PING", "a".repeat(8191 - 1));
    assert_eq!(LineRef::parse(&excessive_tags), Err(ParseError::TooLong));

    let prefix = "PRIVMSG #papo :";
    let maximum_body = format!("{prefix}{}", "x".repeat(512 - 2 - prefix.len()));
    let maximum_line = format!("  @time=2026-10-14T20:15:42.123Z {maximum_body}\r\n");
    assert!(LineRef::parse(&maximum_line).is_ok());
    let excessive_line = format!("  @time=2026-10-14T20:15:42.123Z {maximum_body}x\r\n");
    assert_eq!(LineRef::parse(&excessive_line), Err(ParseError::TooLong));
}

#[test]
fn owns_parsed_lines_without_reparsing() {
    let parsed = LineRef::parse(
        r"@+draft/reply=old;+typing=active;+draft/reply=new\svalue :papo-test!~papo@example.test PRIVMSG #papo :hello there",
    )
    .unwrap();
    let line = parsed.to_owned();
    let borrowed = line.as_ref();

    assert_eq!(borrowed.raw_source(), Some("papo-test!~papo@example.test"));
    assert_eq!(borrowed.command(), Command::Privmsg);
    assert_eq!(
        borrowed.params().collect::<Vec<_>>(),
        ["#papo", "hello there"]
    );
    assert!(matches!(
        borrowed.tags().collect::<Vec<_>>().as_slice(),
        [
            ("+draft/reply", Some(Cow::Borrowed("old"))),
            ("+typing", Some(Cow::Borrowed("active"))),
            ("+draft/reply", Some(Cow::Borrowed("new value"))),
        ]
    ));
}

#[test]
fn builds_common_client_commands() {
    let tagged = Line::privmsg("#papo", "hello there")
        .with_tag("+draft/reply", "abcDEF123")
        .with_tag("+typing", "active")
        .with_tag("+draft/reply", "new-id")
        .with_source("papo-test!~papo@example.test");
    assert!(matches!(
        tagged.as_ref().tags().collect::<Vec<_>>().as_slice(),
        [
            ("+draft/reply", Some(Cow::Borrowed("new-id"))),
            ("+typing", Some(Cow::Borrowed("active"))),
        ]
    ));

    let cases = [
        (Line::notice("papo-test", "maintenance soon"), "NOTICE", 2),
        (Line::tagmsg("#papo"), "TAGMSG", 1),
        (Line::join("#papo"), "JOIN", 1),
        (Line::part("#papo"), "PART", 1),
        (Line::part_with_reason("#papo", "leaving now"), "PART", 2),
        (
            Line::new("CAP").param("REQ").param("message-tags"),
            "CAP",
            2,
        ),
    ];
    for (line, command, param_count) in cases {
        assert_eq!(line.as_ref().raw_command(), command);
        assert_eq!(line.as_ref().len(), param_count);
    }
}

#[test]
fn builds_parameters_one_at_a_time() {
    let chained = Line::new("PRIVMSG").param("#papo").param("release ready");
    assert_eq!(chained.to_string(), "PRIVMSG #papo :release ready");

    let owned = String::from("owned");
    let secret = Secret::new("credential");
    let accepted = Line::new("COMMAND")
        .param("borrowed")
        .param(owned)
        .param(&secret)
        .param(secret);
    assert_eq!(accepted.params().len(), 4);
    assert_eq!(
        format!("{:?}", accepted.params()),
        r#"["borrowed", "owned", [REDACTED], [REDACTED]]"#
    );
}

#[test]
fn parameter_and_wire_traits_observe_exact_values() {
    let line = Line::new("REGISTER")
        .param("alice")
        .param(Secret::new("correct horse battery staple"))
        .with_tag("time", "2026-10-14T20:15:42.123Z")
        .with_tag("account", "alice")
        .with_tag("bot", "")
        .with_source("ergo.test");
    let params = line.params();

    assert_eq!(format!("{}", params[0]), "alice");
    assert_eq!(format!("{}", params[1]), "<redacted>");
    assert!(PartialEq::<str>::eq(&params[0], "alice"));
    assert!(!PartialEq::<str>::eq(&params[0], "bob"));
    assert!(PartialEq::<&str>::eq(&params[0], &"alice"));
    assert!(!PartialEq::<&str>::eq(&params[0], &"bob"));

    let wire = line.encode().unwrap();
    let expected = b"@time=2026-10-14T20:15:42.123Z;account=alice;bot :ergo.test REGISTER alice :correct horse battery staple\r\n";
    assert_eq!(AsRef::<[u8]>::as_ref(&wire), expected);
    assert!(PartialEq::<[u8]>::eq(&wire, expected));
    assert!(!PartialEq::<[u8]>::eq(&wire, b"REGISTER alice wrong\r\n"));
    assert_eq!(
        format!("{wire:?}"),
        "@time=2026-10-14T20:15:42.123Z;account=alice;bot :ergo.test REGISTER alice :<redacted>"
    );
}

#[test]
fn rejects_binary_secret_parameters_before_encoding() {
    assert_eq!(
        Line::new("AUTHENTICATE")
            .param(Secret::from_bytes([0xff, 0x00]))
            .encode(),
        Err(BuildError::InvalidParam(0))
    );
}

#[test]
fn encoded_lines_mask_only_secret_parameters() {
    let ordinary = Line::new("PRIVMSG").param("#papo").param("hi");
    assert_eq!(format!("{ordinary:?}"), "PRIVMSG #papo hi");
    assert_eq!(
        format!("{}", ordinary.encode().unwrap()),
        "PRIVMSG #papo hi"
    );

    let pass = Line::new("PASS").param(Secret::new("hunter2"));
    assert_eq!(format!("{pass:?}"), "PASS <redacted>");
    assert_eq!(format!("{}", pass.encode().unwrap()), "PASS <redacted>");
    assert_eq!(pass.to_line(), Err(BuildError::InvalidParam(0)));

    let payload = Secret::new("x".repeat(44));
    assert_eq!(
        format!(
            "{}",
            Line::new("AUTHENTICATE").param(payload).encode().unwrap()
        ),
        "AUTHENTICATE <redacted, 44 bytes>"
    );
    assert_eq!(
        format!(
            "{}",
            Line::new("WEBIRC")
                .param(Secret::new("password"))
                .param("gateway")
                .param("host")
                .param("address")
                .encode()
                .unwrap()
        ),
        "WEBIRC <redacted> gateway host address"
    );
    assert_eq!(
        format!("{}", "JOIN #secret key".parse::<Line>().unwrap()),
        "JOIN #secret key"
    );
}

#[test]
fn marked_parameters_are_redacted_in_logged_lines() {
    let mut line = ":ergo.test 324 papo #staff +nk hunter2"
        .parse::<Line>()
        .unwrap();
    line.__mark_secret(3);

    assert_eq!(line.get(3), Some("hunter2"));
    assert_eq!(
        line.encode().unwrap().as_ref(),
        b":ergo.test 324 papo #staff +nk hunter2\r\n"
    );
    assert_eq!(
        format!("{line:?}"),
        ":ergo.test 324 papo #staff +nk <redacted>"
    );
}

#[test]
fn masks_multiple_secrets_at_exact_offsets_in_a_realistic_line() {
    let line = Line::new("REGISTER")
        .param("alice")
        .param(Secret::new("invite-code-42"))
        .param(Secret::new("correct horse battery staple"))
        .with_tag("time", "2026-10-14T20:15:42.123Z")
        .with_tag("msgid", "abcDEF123")
        .with_tag("+draft/account", "alice smith")
        .with_source("soju.example");
    let wire = line.encode().unwrap();

    assert_eq!(
        wire.as_ref(),
        b"@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123;+draft/account=alice\\ssmith :soju.example REGISTER alice invite-code-42 :correct horse battery staple\r\n"
    );
    assert_eq!(
        format!("{wire}"),
        r"@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123;+draft/account=alice\ssmith :soju.example REGISTER alice <redacted> :<redacted>"
    );
}

#[test]
fn chooses_middle_and_trailing_parameter_forms_from_the_whole_line() {
    let cases = [
        (
            Line::new("CAP").param("REQ").param("message-tags"),
            "CAP REQ :message-tags\r\n",
        ),
        (Line::new("CAP").param("LS").param("302"), "CAP LS 302\r\n"),
        (
            Line::notice("papo", "maintenance"),
            "NOTICE papo maintenance\r\n",
        ),
        (
            Line::notice("papo", "maintenance soon"),
            "NOTICE papo :maintenance soon\r\n",
        ),
    ];

    for (line, expected) in cases {
        assert_eq!(line.to_line().unwrap(), expected);
    }
}

#[test]
fn passes_ircdocs_line_join_corpus() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-join.json")).unwrap();

    for case in corpus["tests"].as_array().unwrap() {
        let matches = case["matches"].as_array().unwrap();
        let input = matches[0].as_str().unwrap();
        let output = LineRef::parse(input).unwrap().to_owned().to_line().unwrap();
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
    let reply = Line::privmsg("#papo", "hello there")
        .with_tag("+draft/reply", "abcDEF123")
        .with_tag("+reply", "xyz789");
    assert_eq!(
        reply.to_line().unwrap(),
        "@+draft/reply=abcDEF123;+reply=xyz789 PRIVMSG #papo :hello there\r\n"
    );

    let chunk = "dGVzdABwYXBvLXRlc3QAc2VjcmV0".repeat(12);
    assert_eq!(
        Line::new("AUTHENTICATE").param(&chunk).to_line().unwrap(),
        format!("AUTHENTICATE {chunk}\r\n")
    );
    assert_eq!(
        Line::new("CAP")
            .param("REQ")
            .param("message-tags server-time batch labeled-response")
            .to_line()
            .unwrap(),
        "CAP REQ :message-tags server-time batch labeled-response\r\n"
    );
}

#[test]
fn rejects_forbidden_command_and_parameter_bytes() {
    for command in [
        "",
        "@",
        ":source",
        "PRIV MSG",
        "PRIV\rMSG",
        "PRIV\nMSG",
        "PRIV\0MSG",
    ] {
        assert_eq!(
            Line::new(command).param("#papo").param("hello").to_line(),
            Err(BuildError::InvalidCommand)
        );
    }
    for param in ["hello\rthere", "hello\nthere", "hello\0there"] {
        assert_eq!(
            Line::privmsg("#papo", param).to_line(),
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
            Line::new("NOTICE")
                .param("papo-test")
                .param("maintenance")
                .with_source(source)
                .to_line(),
            Err(BuildError::InvalidSource)
        );
    }
    for key in [
        "", "bad key", "bad;key", "bad=key", "bad\rkey", "bad\nkey", "bad\0key",
    ] {
        assert_eq!(
            Line::tagmsg("#papo").with_tag(key, "active").to_line(),
            Err(BuildError::InvalidTagKey)
        );
    }
    assert_eq!(
        Line::tagmsg("#papo")
            .with_tag("+example.test/status", "active\0idle")
            .to_line(),
        Err(BuildError::InvalidTagValue)
    );
    assert_eq!(
        Line::tagmsg("#papo")
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
            Line::new("COMMAND").param(invalid).param("last").to_line(),
            Err(BuildError::InvalidParam(0))
        );
    }

    assert_eq!(
        Line::new("COMMAND").param("").to_line().unwrap(),
        "COMMAND :\r\n"
    );
    assert_eq!(
        Line::new("COMMAND").param("two words").to_line().unwrap(),
        "COMMAND :two words\r\n"
    );
    assert_eq!(
        Line::new("COMMAND").param(":prefixed").to_line().unwrap(),
        "COMMAND ::prefixed\r\n"
    );
    assert_eq!(
        Line::new("COMMAND").param("ordinary").to_line().unwrap(),
        "COMMAND ordinary\r\n"
    );
}

#[test]
fn enforces_client_tag_data_length_at_the_byte_boundary() {
    let fixed = concat!(
        "+draft/reply=abcDEF123;",
        "time=2026-10-14T20:15:42.123Z;",
        "+example.test/pad="
    );
    assert_eq!(fixed.len(), 71);

    for (tag_data_len, expected) in [
        (4093, Ok(())),
        (4094, Ok(())),
        (4095, Err(BuildError::TagsTooLong)),
    ] {
        let line = Line::tagmsg("#papo")
            .with_tag("+draft/reply", "abcDEF123")
            .with_tag("time", "2026-10-14T20:15:42.123Z")
            .with_tag("+example.test/pad", "x".repeat(tag_data_len - 71));
        let result = line.to_line();
        assert_eq!(
            result.as_ref().map(|_| ()).map_err(|error| *error),
            expected,
            "tag data {tag_data_len}"
        );
        if let Ok(encoded) = result {
            let actual = encoded
                .strip_prefix('@')
                .unwrap()
                .split_once(' ')
                .unwrap()
                .0;
            assert_eq!(actual.len(), tag_data_len);
            assert_eq!(encoded.capacity(), encoded.len());
        }
    }
}

#[test]
fn enforces_serialized_body_length_at_the_byte_boundary() {
    let body_prefix = ":soju.example PRIVMSG #papo :deployment ";
    assert_eq!(body_prefix.len(), 40);

    for (body_len, expected) in [
        (511, Ok(())),
        (512, Ok(())),
        (513, Err(BuildError::LineTooLong)),
    ] {
        let text = format!(
            "deployment {}",
            "x".repeat(body_len - 2 - body_prefix.len())
        );
        let line = Line::privmsg("#papo", text)
            .with_tag("time", "2026-10-14T20:15:42.123Z")
            .with_tag("msgid", "abcDEF123")
            .with_tag("account", "alice")
            .with_source("soju.example");
        let result = line.to_line();
        assert_eq!(
            result.as_ref().map(|_| ()).map_err(|error| *error),
            expected,
            "body length {body_len}"
        );
        if let Ok(encoded) = result {
            let body = encoded.split_once(' ').unwrap().1;
            assert_eq!(body.len(), body_len);
            assert_eq!(encoded.capacity(), encoded.len());
        }
    }
}

#[test]
fn displays_each_build_error() {
    let cases = [
        (BuildError::InvalidCommand, "IRC command is invalid"),
        (BuildError::InvalidParam(2), "IRC parameter 2 is invalid"),
        (BuildError::InvalidSource, "IRC source is invalid"),
        (BuildError::InvalidTagKey, "IRC tag key is invalid"),
        (BuildError::InvalidTagValue, "IRC tag value is invalid"),
        (
            BuildError::TagsTooLong,
            "IRC client tag data exceeds its wire limit",
        ),
        (
            BuildError::LineTooLong,
            "IRC line body exceeds its wire limit",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}
