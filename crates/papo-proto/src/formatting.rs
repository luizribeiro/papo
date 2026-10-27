use std::{borrow::Cow, fmt::Write as _, iter::FusedIterator};

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
pub fn strip_formatting(text: &str) -> Cow<'_, str> {
    display_text(text)
}

fn strip_irc_formatting(text: &str) -> Cow<'_, str> {
    if !text.bytes().any(is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(Formatted::new(text).map(|span| span.text).collect())
}

/// Removes terminal control characters and escape sequences from human-readable text.
///
/// Tabs are retained. Other C0 controls, DEL, C1 controls, CSI sequences, and OSC-like
/// control strings are removed. The input is borrowed when it needs no sanitization.
#[must_use]
pub fn display_text(text: &str) -> Cow<'_, str> {
    let formatted = strip_irc_formatting(text);
    let formatted_is_borrowed = matches!(formatted, Cow::Borrowed(_));
    let sanitized = strip_terminal_controls(&formatted);
    if formatted_is_borrowed && matches!(sanitized, Cow::Borrowed(_)) {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(sanitized.into_owned())
    }
}

fn strip_terminal_controls(text: &str) -> Cow<'_, str> {
    #[derive(Clone, Copy)]
    enum Escape {
        Ground,
        Esc,
        Csi,
        String,
        StringEsc,
    }

    let mut clean = String::with_capacity(text.len());
    let mut state = Escape::Ground;
    let mut changed = false;
    for character in text.chars() {
        state = match (state, character) {
            (Escape::Ground, '\t') => {
                clean.push(character);
                Escape::Ground
            }
            (Escape::Ground, '\u{1b}') => {
                changed = true;
                Escape::Esc
            }
            (Escape::Ground, '\u{009b}') => {
                changed = true;
                Escape::Csi
            }
            (Escape::Ground, '\u{0090}' | '\u{0098}' | '\u{009d}' | '\u{009e}' | '\u{009f}') => {
                changed = true;
                Escape::String
            }
            (Escape::Ground, character) if character.is_control() => {
                changed = true;
                Escape::Ground
            }
            (Escape::Ground, character) => {
                clean.push(character);
                Escape::Ground
            }
            (Escape::Esc, '[') => Escape::Csi,
            (Escape::Esc, 'P' | 'X' | ']' | '^' | '_') => Escape::String,
            (Escape::Esc, '\u{1b}' | ' '..='/') | (Escape::Csi, '\u{1b}') => Escape::Esc,
            (Escape::Esc, _)
            | (Escape::Csi, '@'..='~')
            | (Escape::String, '\u{7}' | '\u{009c}')
            | (Escape::StringEsc, '\\') => Escape::Ground,
            (Escape::Csi, _) => Escape::Csi,
            (Escape::String | Escape::StringEsc, '\u{1b}') => Escape::StringEsc,
            (Escape::String | Escape::StringEsc, _) => Escape::String,
        };
    }
    if changed {
        Cow::Owned(clean)
    } else {
        Cow::Borrowed(text)
    }
}

/// A builder for IRC-formatted text.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FormatBuilder {
    output: String,
}

impl FormatBuilder {
    /// Creates an empty formatted-text builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            output: String::new(),
        }
    }

    /// Appends text without changing the active formatting.
    pub fn text(&mut self, text: impl AsRef<str>) -> &mut Self {
        self.output.push_str(text.as_ref());
        self
    }

    /// Toggles bold text.
    pub fn bold(&mut self) -> &mut Self {
        self.control(BOLD)
    }

    /// Toggles italic text.
    pub fn italics(&mut self) -> &mut Self {
        self.control(ITALICS)
    }

    /// Toggles underlined text.
    pub fn underline(&mut self) -> &mut Self {
        self.control(UNDERLINE)
    }

    /// Toggles struck-through text.
    pub fn strikethrough(&mut self) -> &mut Self {
        self.control(STRIKETHROUGH)
    }

    /// Toggles monospace text.
    pub fn monospace(&mut self) -> &mut Self {
        self.control(MONOSPACE)
    }

    /// Toggles reversed foreground and background colors.
    pub fn reverse(&mut self) -> &mut Self {
        self.control(REVERSE)
    }

    /// Selects palette foreground and optional background colors.
    ///
    /// Both values must be in the IRC palette range `0..=99`.
    ///
    /// # Panics
    ///
    /// Panics when either value is greater than 99.
    pub fn color(&mut self, foreground: u8, background: Option<u8>) -> &mut Self {
        assert!(foreground <= 99, "IRC palette colors are at most 99");
        assert!(
            background.is_none_or(|color| color <= 99),
            "IRC palette colors are at most 99"
        );
        self.output.push(char::from(COLOR));
        write!(self.output, "{foreground:02}").expect("writing to a String cannot fail");
        if let Some(background) = background {
            write!(self.output, ",{background:02}").expect("writing to a String cannot fail");
        }
        self
    }

    /// Selects six-digit RGB foreground and optional background colors.
    ///
    /// # Panics
    ///
    /// Panics when either value is greater than `0xffffff`.
    pub fn hex_color(&mut self, foreground: u32, background: Option<u32>) -> &mut Self {
        assert!(foreground <= 0x00ff_ffff, "RGB colors are 24-bit values");
        assert!(
            background.is_none_or(|color| color <= 0x00ff_ffff),
            "RGB colors are 24-bit values"
        );
        self.output.push(char::from(HEX_COLOR));
        write!(self.output, "{foreground:06x}").expect("writing to a String cannot fail");
        if let Some(background) = background {
            write!(self.output, ",{background:06x}").expect("writing to a String cannot fail");
        }
        self
    }

    /// Resets the foreground and background colors.
    pub fn reset_colors(&mut self) -> &mut Self {
        self.control(COLOR)
    }

    /// Resets every active style and color.
    pub fn reset(&mut self) -> &mut Self {
        self.control(RESET)
    }

    /// Returns the formatted text built so far.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.output
    }

    /// Finishes the builder and returns its formatted text.
    #[must_use]
    pub fn finish(self) -> String {
        self.output
    }

    fn control(&mut self, control: u8) -> &mut Self {
        self.output.push(char::from(control));
        self
    }
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
