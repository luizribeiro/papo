/// One byte-bounded portion of outgoing message text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment<'a> {
    /// Text to send, excluding a following embedded newline separator.
    pub text: &'a str,
    /// The boundary between this segment and the next one.
    pub boundary: SegmentBoundary,
}

/// What follows one outgoing text segment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SegmentBoundary {
    /// The input ends after this segment.
    End,
    /// Papo split the input to fit a byte limit.
    Split,
    /// The user supplied a newline after this segment.
    Newline,
}

/// Splits message text into segments of at most `budget` bytes.
///
/// Spaces are retained at the end of the preceding segment when possible. Words
/// longer than the budget are divided at UTF-8 boundaries. Empty segments preserve
/// user-supplied blank lines. The second tuple element is the unsplit suffix when no
/// UTF-8 character fits the budget.
#[must_use]
pub fn split_text(text: &str, budget: usize) -> (Vec<Segment<'_>>, &str) {
    let mut segments = Vec::new();
    if budget == 0 {
        return (segments, text);
    }
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        let line_boundary = if lines.peek().is_some() {
            SegmentBoundary::Newline
        } else {
            SegmentBoundary::End
        };
        if line.is_empty() {
            if !text.is_empty() {
                segments.push(Segment {
                    text: line,
                    boundary: line_boundary,
                });
            }
            continue;
        }
        let mut remaining = line;
        while !remaining.is_empty() {
            if remaining.len() <= budget {
                segments.push(Segment {
                    text: remaining,
                    boundary: line_boundary,
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
                boundary: SegmentBoundary::Split,
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
