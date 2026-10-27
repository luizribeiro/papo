//! Integration tests for IRC text formatting.

use std::borrow::Cow;

use papo_proto::{Color, FormatBuilder, Formatted, Span, Style, display_text, strip_formatting};

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
        assert_eq!(strip_formatting(input), expected);
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
    assert!(matches!(strip_formatting(""), Cow::Borrowed(_)));
    assert_eq!(strip_formatting("\x02\x1d\x03"), "");
}

#[test]
fn strip_borrows_only_unformatted_text() {
    assert!(matches!(
        strip_formatting("hello from Ergo"),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        strip_formatting("hello \x02from Ergo"),
        Cow::Owned(_)
    ));
}

#[test]
fn display_text_removes_terminal_and_irc_control_sequences() {
    for (input, expected) in [
        ("\x1b[2Jhello", "hello"),
        ("before\x1b]52;c;Y2xpcGJvYXJk\x07after", "beforeafter"),
        ("\u{009b}31mred\u{009b}0m", "red"),
        ("safe\x1bPunterminated device payload", "safe"),
        ("\x02bold\x02 and \x0304,12color\x03", "bold and color"),
        (
            "one\rforged\nline\u{007f}\u{0085}two\ttab",
            "oneforgedlinetwo\ttab",
        ),
    ] {
        assert_eq!(display_text(input), expected);
    }
    assert!(matches!(display_text("hello from Ergo"), Cow::Borrowed(_)));
}

#[test]
fn stripped_multiline_text_retains_reconstructed_separators() {
    assert_eq!(
        strip_formatting("first deployment line\nsecond \x02deployment\x02 line"),
        "first deployment line\nsecond deployment line"
    );
    assert_eq!(
        display_text("first deployment line\nsecond deployment line"),
        "first deployment linesecond deployment line"
    );
}

#[test]
fn built_text_round_trips_through_the_span_parser() {
    let mut builder = FormatBuilder::new();
    builder
        .text("IRC ")
        .bold()
        .italics()
        .underline()
        .strikethrough()
        .monospace()
        .reverse()
        .color(4, Some(12))
        .text("formatted")
        .hex_color(0x00ff_0080, Some(0x0000_ff00))
        .text(" RGB")
        .reset_colors()
        .text(" plain colors")
        .reset()
        .text(" done");

    let output = builder.finish();
    let parsed = spans(&output);
    assert_eq!(
        strip_formatting(&output),
        "IRC formatted RGB plain colors done"
    );
    assert_eq!(parsed[1].style.fg, Some(Color::Palette(4)));
    assert_eq!(parsed[1].style.bg, Some(Color::Palette(12)));
    assert!(parsed[1].style.bold && parsed[1].style.italics);
    assert_eq!(parsed[2].style.fg, Some(Color::Hex(0x00ff_0080)));
    assert_eq!(parsed[2].style.bg, Some(Color::Hex(0x0000_ff00)));
    assert_eq!(parsed[3].style.fg, None);
    assert!(parsed[3].style.bold);
    assert_eq!(parsed[4].style, Style::default());
}

#[test]
fn builder_writes_two_digit_palette_colors_before_digit_text() {
    let mut builder = FormatBuilder::new();
    builder.color(3, Some(7)).text("13 channels");

    assert_eq!(builder.as_str(), "\x0303,0713 channels");
    assert_eq!(spans(builder.as_str())[0].text, "13 channels");
    assert_eq!(spans(builder.as_str())[0].style.fg, Some(Color::Palette(3)));
    assert_eq!(spans(builder.as_str())[0].style.bg, Some(Color::Palette(7)));
}

#[test]
#[should_panic(expected = "IRC palette colors are at most 99")]
fn builder_rejects_palette_colors_outside_two_digits() {
    FormatBuilder::new().color(100, None);
}

#[test]
#[should_panic(expected = "IRC palette colors are at most 99")]
fn builder_rejects_palette_backgrounds_outside_two_digits() {
    FormatBuilder::new().color(3, Some(100));
}

#[test]
#[should_panic(expected = "RGB colors are 24-bit values")]
fn builder_rejects_hex_colors_larger_than_six_digits() {
    FormatBuilder::new().hex_color(0x0100_0000, None);
}

#[test]
#[should_panic(expected = "RGB colors are 24-bit values")]
fn builder_rejects_hex_backgrounds_larger_than_six_digits() {
    FormatBuilder::new().hex_color(0x00ff_0080, Some(0x0100_0000));
}
