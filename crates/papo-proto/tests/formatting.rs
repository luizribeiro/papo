//! Integration tests for IRC text formatting.

use std::borrow::Cow;

use papo_proto::{Formatted, Span, Style, strip};

fn spans(text: &str) -> Vec<Span<'_>> {
    Formatted::new(text).collect()
}

#[test]
fn tracks_every_toggle_and_reset() {
    let input = "plain\x02b\x1di\x1fu\x1es\x11m\x16r\x0fplain";
    let parsed = spans(input);

    assert_eq!(parsed[0].style, Style::default());
    assert!(parsed[1].style.bold);
    assert!(parsed[2].style.italics);
    assert!(parsed[3].style.underline);
    assert!(parsed[4].style.strikethrough);
    assert!(parsed[5].style.monospace);
    assert!(parsed[6].style.reverse);
    assert_eq!(parsed[7].style, Style::default());
}

#[test]
fn handles_empty_and_control_only_input() {
    assert!(spans("").is_empty());
    assert!(spans("\x02\x1d\x0f").is_empty());
    assert!(matches!(strip(""), Cow::Borrowed(_)));
    assert_eq!(strip("\x02\x1d\x0f"), "");
}

#[test]
fn strip_borrows_only_unformatted_text() {
    assert!(matches!(strip("hello from Ergo"), Cow::Borrowed(_)));
    assert!(matches!(strip("hello \x02from Ergo"), Cow::Owned(_)));
}
