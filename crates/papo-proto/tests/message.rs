//! Integration tests for borrowed IRC message parsing.

use std::{borrow::Cow, collections::BTreeMap};

use papo_proto::{MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, MessageRef, ParseError};
use serde_json::Value;

#[test]
fn passes_ircdocs_message_split_corpus() {
    let corpus: Value =
        serde_json::from_str(include_str!("data/parser-tests/msg-split.json")).unwrap();

    for case in corpus["tests"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let expected = &case["atoms"];
        let message = MessageRef::parse(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));

        assert_eq!(message.source(), expected["source"].as_str(), "{input:?}");
        assert_eq!(
            message.command(),
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
fn parses_realistic_ergo_lines() {
    let message = MessageRef::parse(
        "@time=2026-10-14T20:15:42.123Z;msgid=abcDEF123 :alice!~u@example.test PRIVMSG #papo :hello there\r\n",
    )
    .unwrap();
    assert_eq!(message.source(), Some("alice!~u@example.test"));
    assert_eq!(message.command(), "PRIVMSG");
    assert!(message.command_is("privmsg"));
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
    assert_eq!(authenticate.source(), None);
    assert_eq!(authenticate.param(0), Some("+"));

    let ping = MessageRef::parse("PING").unwrap();
    assert!(ping.is_empty());

    let forced_trailing = MessageRef::parse(":ergo.test 323 papo-test :").unwrap();
    assert_eq!(forced_trailing.param(1), Some(""));
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
    assert_eq!(message.source(), Some("alice!~u@example.test"));
    assert_eq!(message.command(), "PRIVMSG");
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
fn enforces_tag_section_length_at_the_byte_boundary() {
    let maximum = format!("@{} PING", "a".repeat(MAX_TAG_SECTION_LEN - 2));
    assert!(MessageRef::parse(&maximum).is_ok());

    let over_limit = format!("@{} PING", "a".repeat(MAX_TAG_SECTION_LEN - 1));
    assert_eq!(MessageRef::parse(&over_limit), Err(ParseError::TooLong));
}

#[test]
fn enforces_message_length_at_the_byte_boundary() {
    let prefix = "PRIVMSG #papo :";
    let maximum_body = format!("{prefix}{}", "x".repeat(MAX_MESSAGE_LEN - 2 - prefix.len()));
    let maximum = format!("@time=2026-10-14T20:15:42.123Z {maximum_body}\r\n");
    assert!(MessageRef::parse(&maximum).is_ok());

    let over_limit = format!("{maximum_body}x\r\n");
    assert_eq!(MessageRef::parse(&over_limit), Err(ParseError::TooLong));
}

#[test]
fn enforces_both_limits_after_leading_spaces() {
    let maximum_tags = format!("  @{} PING", "a".repeat(MAX_TAG_SECTION_LEN - 2));
    assert!(MessageRef::parse(&maximum_tags).is_ok());
    let excessive_tags = format!("  @{} PING", "a".repeat(MAX_TAG_SECTION_LEN - 1));
    assert_eq!(MessageRef::parse(&excessive_tags), Err(ParseError::TooLong));

    let prefix = "PRIVMSG #papo :";
    let maximum_body = format!("{prefix}{}", "x".repeat(MAX_MESSAGE_LEN - 2 - prefix.len()));
    let maximum_message = format!("  @time=2026-10-14T20:15:42.123Z {maximum_body}\r\n");
    assert!(MessageRef::parse(&maximum_message).is_ok());
    let excessive_message = format!("  @time=2026-10-14T20:15:42.123Z {maximum_body}x\r\n");
    assert_eq!(
        MessageRef::parse(&excessive_message),
        Err(ParseError::TooLong)
    );
}
