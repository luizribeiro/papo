use std::{borrow::Cow, iter::FusedIterator};

const BOLD: u8 = 0x02;
const COLOR: u8 = 0x03;
const HEX_COLOR: u8 = 0x04;
const RESET: u8 = 0x0f;
const MONOSPACE: u8 = 0x11;
const REVERSE: u8 = 0x16;
const ITALICS: u8 = 0x1d;
const STRIKETHROUGH: u8 = 0x1e;
const UNDERLINE: u8 = 0x1f;

/// A color selected by an IRC formatting code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Color {
    /// A numbered color in the IRC palette.
    Palette(u8),
    /// A 24-bit RGB color written as six hexadecimal digits.
    Hex(u32),
}

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
    /// The foreground color, when explicitly selected.
    pub fg: Option<Color>,
    /// The background color, when explicitly selected.
    pub bg: Option<Color>,
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
                fg: None,
                bg: None,
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
            COLOR => self.consume_color(false),
            HEX_COLOR => self.consume_color(true),
            _ => unreachable!("only formatting controls are selected"),
        }
    }

    fn consume_color(&mut self, hex: bool) {
        let digits = if hex { 6 } else { 2 };
        let parsed = take_digits(self.remaining, digits, hex);
        let Some((value, consumed)) = parsed else {
            // Fewer than six hex digits cannot name a color, so they reset like a bare ^C.
            self.style.fg = None;
            self.style.bg = None;
            return;
        };
        self.remaining = &self.remaining[consumed..];
        self.style.fg = Some(parsed_color(value, hex));

        let Some(after_comma) = self.remaining.strip_prefix(',') else {
            return;
        };
        if let Some((value, consumed)) = take_digits(after_comma, digits, hex) {
            self.remaining = &after_comma[consumed..];
            self.style.bg = Some(parsed_color(value, hex));
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
        BOLD | COLOR
            | HEX_COLOR
            | RESET
            | MONOSPACE
            | REVERSE
            | ITALICS
            | STRIKETHROUGH
            | UNDERLINE
    )
}

fn take_digits(input: &str, maximum: usize, hex: bool) -> Option<(u32, usize)> {
    let count = input
        .bytes()
        .take(maximum)
        .take_while(|byte| {
            if hex {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
        .count();
    if count == 0 || (hex && count != maximum) {
        return None;
    }
    u32::from_str_radix(&input[..count], if hex { 16 } else { 10 })
        .ok()
        .map(|value| (value, count))
}

fn parsed_color(value: u32, hex: bool) -> Color {
    if hex {
        Color::Hex(value)
    } else {
        Color::Palette(u8::try_from(value).expect("two decimal digits fit in u8"))
    }
}
