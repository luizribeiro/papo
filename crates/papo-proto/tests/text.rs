//! Outgoing text splitting coverage.

use SegmentBoundary::{End, Newline, Split};
use papo_proto::{SegmentBoundary, split_text};

fn parts(text: &str, budget: usize) -> Vec<(&str, SegmentBoundary)> {
    let (segments, leftover) = split_text(text, budget);
    assert!(leftover.is_empty());
    segments
        .into_iter()
        .map(|segment| (segment.text, segment.boundary))
        .collect()
}

#[test]
fn reports_a_suffix_when_no_utf8_character_fits() {
    let (segments, leftover) = split_text("hi 🎉 there", 3);
    assert_eq!(
        segments,
        [papo_proto::Segment {
            text: "hi ",
            boundary: Split
        }]
    );
    assert_eq!(leftover, "🎉 there");
    assert_eq!(
        segments
            .iter()
            .map(|segment| segment.text)
            .chain([leftover])
            .collect::<String>(),
        "hi 🎉 there"
    );
}

#[test]
fn respects_exact_byte_boundaries() {
    assert_eq!(parts("hello", 5), vec![("hello", End)]);
    assert_eq!(parts("hello!", 5), vec![("hello", Split), ("!", End)]);
}

#[test]
fn hard_splits_at_utf8_boundaries() {
    assert_eq!(
        parts("abcédef", 4),
        vec![("abc", Split), ("éde", Split), ("f", End)]
    );
}

#[test]
fn marks_each_hard_split_before_a_word_continuation() {
    assert_eq!(
        parts("supercalifragilistic", 5),
        vec![
            ("super", Split),
            ("calif", Split),
            ("ragil", Split),
            ("istic", End),
        ]
    );
}

#[test]
fn prefers_spaces_and_preserves_them() {
    assert_eq!(parts("one two", 6), vec![("one ", Split), ("two", End)]);
    assert_eq!(parts(" abc ", 4), vec![(" ", Split), ("abc ", End)]);
}

#[test]
fn embedded_newlines_preserve_blank_lines_and_the_end() {
    assert_eq!(
        parts("first\n\nsecond\n", 20),
        vec![
            ("first", Newline),
            ("", Newline),
            ("second", Newline),
            ("", End),
        ]
    );
    assert_eq!(parts("", 20), []);
}

#[test]
fn a_space_at_the_budget_edge_is_not_a_word_continuation() {
    assert_eq!(
        parts("hello world", 6),
        vec![("hello ", Split), ("world", End)]
    );
}
