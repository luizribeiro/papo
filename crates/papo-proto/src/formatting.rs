use std::{borrow::Cow, iter::FusedIterator};

const BOLD: u8 = 0x02;
const RESET: u8 = 0x0f;
const MONOSPACE: u8 = 0x11;
const REVERSE: u8 = 0x16;
const ITALICS: u8 = 0x1d;
const STRIKETHROUGH: u8 = 0x1e;
const UNDERLINE: u8 = 0x1f;

/// The formatting active for a span of text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Style {
    /// Whether bold text is active.
    pub bold: bool,
    /// Whether italic text is active.
    pub italics: bool,
    /// Whether underlined text is active.
    pub underline: bool,
    /// Whether struck-through text is active.
    pub strikethrough: bool,
    /// Whether monospace text is active.
    pub monospace: bool,
    /// Whether foreground and background colors are reversed.
    pub reverse: bool,
}

/// A non-empty piece of text and the formatting active for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span<'a> {
    /// The text without formatting control codes.
    pub text: &'a str,
    /// The formatting active for the text.
    pub style: Style,
}

/// An iterator over the formatted spans in an IRC text value.
#[derive(Clone, Debug)]
pub struct Formatted<'a> {
    remaining: &'a str,
    style: Style,
}

impl<'a> Formatted<'a> {
    /// Creates a span iterator over `text`.
    #[must_use]
    pub const fn new(text: &'a str) -> Self {
        Self {
            remaining: text,
            style: Style {
                bold: false,
                italics: false,
                underline: false,
                strikethrough: false,
                monospace: false,
                reverse: false,
            },
        }
    }

    fn consume_control(&mut self) {
        let control = self.remaining.as_bytes()[0];
        self.remaining = &self.remaining[1..];
        match control {
            BOLD => self.style.bold = !self.style.bold,
            ITALICS => self.style.italics = !self.style.italics,
            UNDERLINE => self.style.underline = !self.style.underline,
            STRIKETHROUGH => self.style.strikethrough = !self.style.strikethrough,
            MONOSPACE => self.style.monospace = !self.style.monospace,
            REVERSE => self.style.reverse = !self.style.reverse,
            RESET => self.style = Style::default(),
            _ => unreachable!("only formatting controls are selected"),
        }
    }
}

impl<'a> Iterator for Formatted<'a> {
    type Item = Span<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let index = self.remaining.bytes().position(is_control);
            match index {
                Some(0) => self.consume_control(),
                Some(index) => {
                    let (text, remaining) = self.remaining.split_at(index);
                    self.remaining = remaining;
                    return Some(Span {
                        text,
                        style: self.style,
                    });
                }
                None if self.remaining.is_empty() => return None,
                None => {
                    let text = std::mem::take(&mut self.remaining);
                    return Some(Span {
                        text,
                        style: self.style,
                    });
                }
            }
        }
    }
}

impl FusedIterator for Formatted<'_> {}

/// Removes IRC formatting codes from `text`.
///
/// The input is borrowed when it contains no recognized formatting control byte.
#[must_use]
pub fn strip(text: &str) -> Cow<'_, str> {
    if !text.bytes().any(is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(Formatted::new(text).map(|span| span.text).collect())
}

fn is_control(byte: u8) -> bool {
    matches!(
        byte,
        BOLD | RESET | MONOSPACE | REVERSE | ITALICS | STRIKETHROUGH | UNDERLINE
    )
}
