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
/// concatenation. Embedded newlines always form boundaries and are discarded. The
/// second tuple element is the unsplit suffix when no UTF-8 character fits the budget.
#[must_use]
pub fn split_text(text: &str, budget: usize) -> (Vec<Segment<'_>>, &str) {
    let mut segments = Vec::new();
    if budget == 0 {
        return (segments, text);
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
                let offset = remaining.as_ptr() as usize - text.as_ptr() as usize;
                return (segments, &text[offset..]);
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
    (segments, &text[text.len()..])
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
