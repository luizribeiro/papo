//! Integration tests for IRC text formatting.

use std::borrow::Cow;

use papo_proto::{Color, Formatted, Span, Style, strip};

fn spans(text: &str) -> Vec<Span<'_>> {
    Formatted::new(text).collect()
}

#[test]
fn parses_the_formatting_spec_examples() {
    let cases = [
        (
            "I love \x033IRC! \x03It is the \x037best protocol ever!",
            "I love IRC! It is the best protocol ever!",
        ),
        (
            "This is a \x1d\x0313,9cool \x03message",
            "This is a cool message",
        ),
        ("IRC \x02is \x034,12so \x03great\x0f!", "IRC is so great!"),
        (
            "Rules: Don't spam 5\x0313,8,6\x03,7,8, and especially not \x029\x02\x1d!",
            "Rules: Don't spam 5,6,7,8, and especially not 9!",
        ),
    ];

    for (input, expected) in cases {
        assert_eq!(strip(input), expected);
        assert_eq!(
            spans(input)
                .iter()
                .map(|span| span.text)
                .collect::<String>(),
            expected
        );
    }
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
fn parses_palette_colors_without_eating_text() {
    let parsed = spans("\x03123 \x039x \x034,12colors \x03,comma");

    assert_eq!(
        parsed[0],
        Span {
            text: "3 ",
            style: Style {
                fg: Some(Color::Palette(12)),
                ..Style::default()
            }
        }
    );
    assert_eq!(parsed[1].style.fg, Some(Color::Palette(9)));
    assert_eq!(
        parsed[2].style,
        Style {
            fg: Some(Color::Palette(4)),
            bg: Some(Color::Palette(12)),
            ..Style::default()
        }
    );
    assert_eq!(
        parsed[3],
        Span {
            text: ",comma",
            style: Style::default()
        }
    );
}

#[test]
fn hex_colors_require_exactly_six_digits() {
    let parsed = spans("\x04ff0080pink \x0400ff00,112233both \x04abcdeZ");

    assert_eq!(parsed[0].style.fg, Some(Color::Hex(0x00ff_0080)));
    assert_eq!(
        parsed[1].style,
        Style {
            fg: Some(Color::Hex(0x0000_ff00)),
            bg: Some(Color::Hex(0x0011_2233)),
            ..Style::default()
        }
    );
    assert_eq!(
        parsed[2],
        Span {
            text: "abcdeZ",
            style: Style::default()
        }
    );
}

#[test]
fn preserves_background_when_only_foreground_changes() {
    let parsed = spans("\x0304,12red on blue \x0303green on blue");

    assert_eq!(parsed[1].style.bg, Some(Color::Palette(12)));
    assert_eq!(parsed[1].style.fg, Some(Color::Palette(3)));
}

#[test]
fn handles_empty_and_control_only_input() {
    assert!(spans("").is_empty());
    assert!(spans("\x02\x1d\x03\x04\x0f").is_empty());
    assert!(matches!(strip(""), Cow::Borrowed(_)));
    assert_eq!(strip("\x02\x1d\x03"), "");
}

#[test]
fn strip_borrows_only_unformatted_text() {
    assert!(matches!(strip("hello from Ergo"), Cow::Borrowed(_)));
    assert!(matches!(strip("hello \x02from Ergo"), Cow::Owned(_)));
}
