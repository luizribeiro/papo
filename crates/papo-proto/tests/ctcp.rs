//! Integration tests for CTCP message bodies.

use papo_proto::Ctcp;

#[test]
fn parses_action_with_or_without_the_final_delimiter() {
    let expected = Ctcp {
        command: "ACTION",
        params: Some("writes the best specifications!"),
    };

    assert_eq!(
        Ctcp::parse("\x01ACTION writes the best specifications!\x01"),
        Some(expected)
    );
    assert_eq!(
        Ctcp::parse("\x01ACTION writes the best specifications!"),
        Some(expected)
    );
}

#[test]
fn parses_version_without_parameters() {
    assert_eq!(
        Ctcp::parse("\x01VERSION\x01"),
        Some(Ctcp {
            command: "VERSION",
            params: None,
        })
    );
}

#[test]
fn rejects_a_lone_delimiter_and_plain_text() {
    assert_eq!(Ctcp::parse("\x01"), None);
    assert_eq!(Ctcp::parse("hello from Ergo"), None);
}

#[test]
fn rejects_invalid_commands_and_parameters() {
    assert_eq!(Ctcp::parse("\x01ACTION \x01"), None);
    assert_eq!(Ctcp::parse("\x01VER\x01SION\x01"), None);
    assert_eq!(Ctcp::parse("\x01PING 1473523796\n918320\x01"), None);
}

#[test]
fn builds_action_and_parameterless_queries_with_a_final_delimiter() {
    let action = Ctcp::action("writes the best specifications!").unwrap();
    let version = Ctcp::build("VERSION", None).unwrap();

    assert_eq!(action, "\x01ACTION writes the best specifications!\x01");
    assert_eq!(version, "\x01VERSION\x01");
    assert_eq!(
        Ctcp::parse(&action).unwrap().params,
        Some("writes the best specifications!")
    );
    assert_eq!(Ctcp::parse(&version).unwrap().params, None);
}

#[test]
fn builder_rejects_an_empty_command() {
    assert_eq!(Ctcp::build("", None), None);
}

#[test]
fn builder_rejects_delimiters_in_parameters() {
    assert_eq!(Ctcp::build("PING", Some("1473523796\x01918320")), None);
}
