/// One byte-bounded portion of outgoing message text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment<'a> {
    /// Text to send, excluding an embedded newline separator.
    pub text: &'a str,
    /// Whether the following segment continues this segment without a separator.
    pub concat: bool,
}

/// Splits message text into nonempty segments of at most `budget` bytes.
///
/// Spaces are retained at the end of the preceding segment when possible. Words
/// longer than the budget are divided at UTF-8 boundaries and marked for
/// concatenation. Embedded newlines always form boundaries and are discarded.
#[must_use]
pub fn split_text(text: &str, budget: usize) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    if budget == 0 {
        return segments;
    }
    for line in text.split('\n') {
        let mut remaining = line;
        while !remaining.is_empty() {
            if remaining.len() <= budget {
                segments.push(Segment {
                    text: remaining,
                    concat: false,
                });
                break;
            }

            let boundary = floor_char_boundary(remaining, budget);
            if boundary == 0 {
                break;
            }
            let split = remaining[..boundary]
                .rfind(' ')
                .map_or(boundary, |space| space + 1);
            segments.push(Segment {
                text: &remaining[..split],
                concat: split == boundary && !remaining[..split].ends_with(' '),
            });
            remaining = &remaining[split..];
        }
    }
    segments
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
