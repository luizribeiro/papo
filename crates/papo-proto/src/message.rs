use std::{borrow::Cow, error::Error, fmt, iter::FusedIterator, str::Utf8Error};

/// Maximum byte length of an IRC message excluding its tag section but including CRLF.
///
/// This limit is defined by the [Modern IRC client protocol].
///
/// [Modern IRC client protocol]: https://modern.ircdocs.horse/#message-format
pub const MAX_MESSAGE_LEN: usize = 512;

/// Maximum byte length of a received IRC tag section, including `@` and its trailing space.
///
/// This limit is defined by the [IRCv3 message-tags specification].
///
/// [IRCv3 message-tags specification]: https://ircv3.net/specs/extensions/message-tags#size-limit
pub const MAX_TAG_SECTION_LEN: usize = 8191;

/// A failure to parse an IRC message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// The line has no content before its line ending.
    EmptyLine,
    /// The line contains tags, a source, or spaces but no command.
    MissingCommand,
    /// The byte input is not valid UTF-8.
    InvalidUtf8,
    /// The tag section or the rest of the message exceeds its wire limit.
    TooLong,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyLine => "IRC line is empty",
            Self::MissingCommand => "IRC line has no command",
            Self::InvalidUtf8 => "IRC line is not valid UTF-8",
            Self::TooLong => "IRC line exceeds its wire length limit",
        })
    }
}

impl Error for ParseError {}

impl From<Utf8Error> for ParseError {
    fn from(_: Utf8Error) -> Self {
        Self::InvalidUtf8
    }
}

/// A zero-copy view of one IRC message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageRef<'a> {
    tags: Option<&'a str>,
    source: Option<&'a str>,
    command: &'a str,
    params: &'a str,
}

impl<'a> MessageRef<'a> {
    /// Parses one UTF-8 IRC line, with or without a trailing line ending.
    ///
    /// Both CRLF and a bare LF are accepted as line endings.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::EmptyLine`] for an empty line and
    /// [`ParseError::MissingCommand`] when no command is present, and
    /// [`ParseError::TooLong`] when either wire length limit is exceeded.
    pub fn parse(input: &'a str) -> Result<Self, ParseError> {
        let line = input
            .strip_suffix("\r\n")
            .or_else(|| input.strip_suffix('\n'))
            .unwrap_or(input);
        if line.is_empty() {
            return Err(ParseError::EmptyLine);
        }

        let mut rest = line.trim_start_matches(' ');
        check_wire_lengths(rest)?;
        let tags = take_field(&mut rest, '@')?;
        let source = take_field(&mut rest, ':')?;

        if rest.is_empty() {
            return Err(ParseError::MissingCommand);
        }
        let (command, params) = rest.split_once(' ').unwrap_or((rest, ""));
        if command.is_empty() {
            return Err(ParseError::MissingCommand);
        }

        Ok(Self {
            tags,
            source,
            command,
            params,
        })
    }

    /// Parses one IRC line from bytes, rejecting invalid UTF-8.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidUtf8`] for non-UTF-8 input, in addition to
    /// the structural errors returned by [`Self::parse`].
    pub fn parse_bytes(input: &'a [u8]) -> Result<Self, ParseError> {
        Self::parse(std::str::from_utf8(input)?)
    }

    /// Returns the message tags in wire order.
    ///
    /// Values are unescaped lazily. Tags without values yield `None`.
    #[must_use]
    pub fn tags(&self) -> Tags<'a> {
        Tags {
            inner: self.tags.map(|tags| tags.split(';')),
        }
    }

    /// Returns the last value for the case-sensitive tag key.
    ///
    /// A missing value and an absent key both return `None`.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<Cow<'a, str>> {
        self.tags()
            .filter_map(|(candidate, value)| (candidate == key).then_some(value))
            .last()
            .flatten()
    }

    /// Returns the source without its leading colon, when present.
    #[must_use]
    pub const fn source(&self) -> Option<&'a str> {
        self.source
    }

    /// Returns the command with its original spelling.
    #[must_use]
    pub const fn command(&self) -> &'a str {
        self.command
    }

    /// Reports whether the command matches using ASCII case-insensitive comparison.
    #[must_use]
    pub fn command_is(&self, command: &str) -> bool {
        self.command.eq_ignore_ascii_case(command)
    }

    /// Returns the parameters in wire order.
    #[must_use]
    pub fn params(&self) -> Params<'a> {
        Params {
            remaining: self.params,
        }
    }

    /// Returns the parameter at `index`, if present.
    #[must_use]
    pub fn param(&self, index: usize) -> Option<&'a str> {
        self.params().nth(index)
    }

    /// Returns the number of parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.params().count()
    }

    /// Reports whether the message has no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.params().next().is_none()
    }
}

/// An iterator over the tags of a borrowed IRC message.
#[derive(Clone, Debug)]
pub struct Tags<'a> {
    inner: Option<std::str::Split<'a, char>>,
}

impl<'a> Iterator for Tags<'a> {
    type Item = (&'a str, Option<Cow<'a, str>>);

    fn next(&mut self) -> Option<Self::Item> {
        let tag = self.inner.as_mut()?.next()?;
        let (key, value) = tag.split_once('=').map_or((tag, None), |(key, value)| {
            (key, (!value.is_empty()).then_some(value))
        });
        Some((key, value.map(unescape_tag)))
    }
}

impl FusedIterator for Tags<'_> {}

/// An iterator over the parameters of a borrowed IRC message.
#[derive(Clone, Debug)]
pub struct Params<'a> {
    remaining: &'a str,
}

impl<'a> Iterator for Params<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let remaining = self.remaining.trim_start_matches(' ');
        if remaining.is_empty() {
            self.remaining = "";
            return None;
        }
        if let Some(trailing) = remaining.strip_prefix(':') {
            self.remaining = "";
            return Some(trailing);
        }
        let (param, rest) = remaining.split_once(' ').unwrap_or((remaining, ""));
        self.remaining = rest;
        Some(param)
    }
}

impl FusedIterator for Params<'_> {}

fn take_field<'a>(rest: &mut &'a str, prefix: char) -> Result<Option<&'a str>, ParseError> {
    let Some(prefixed) = rest.strip_prefix(prefix) else {
        return Ok(None);
    };
    let Some((field, after_field)) = prefixed.split_once(' ') else {
        return Err(ParseError::MissingCommand);
    };
    *rest = after_field.trim_start_matches(' ');
    Ok(Some(field))
}

fn check_wire_lengths(line: &str) -> Result<(), ParseError> {
    let tag_section_len = line.strip_prefix('@').map_or(0, |tagged| {
        tagged.find(' ').map_or(0, |separator| separator + 2)
    });
    // Every accepted terminator occupies the protocol's canonical CRLF budget.
    let message_len = line.len() - tag_section_len + 2;
    if tag_section_len > MAX_TAG_SECTION_LEN || message_len > MAX_MESSAGE_LEN {
        return Err(ParseError::TooLong);
    }
    Ok(())
}

fn unescape_tag(value: &str) -> Cow<'_, str> {
    if !value.contains('\\') {
        return Cow::Borrowed(value);
    }

    let mut unescaped = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        if let Some(escaped) = chars.next() {
            unescaped.push(match escaped {
                ':' => ';',
                's' => ' ',
                '\\' => '\\',
                'r' => '\r',
                'n' => '\n',
                other => other,
            });
        }
    }
    Cow::Owned(unescaped)
}
