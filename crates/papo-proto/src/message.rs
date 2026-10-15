use std::{borrow::Cow, error::Error, fmt, iter::FusedIterator, slice, str::Utf8Error};

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
///
/// New variants may be added as more protocol validation is introduced.
///
/// ```compile_fail
/// use papo_proto::ParseError;
///
/// match ParseError::EmptyLine {
///     ParseError::EmptyLine => {}
///     ParseError::MissingCommand => {}
///     ParseError::InvalidUtf8 => {}
///     ParseError::TooLong => {}
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
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
    tags: TagStorage<'a>,
    source: Option<&'a str>,
    command: &'a str,
    params: ParamStorage<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TagStorage<'a> {
    Wire(Option<&'a str>),
    Owned(&'a [(String, Option<String>)]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParamStorage<'a> {
    Wire(&'a str),
    Owned(&'a [String]),
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
            tags: TagStorage::Wire(tags),
            source,
            command,
            params: ParamStorage::Wire(params),
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
            inner: match self.tags {
                TagStorage::Wire(tags) => TagIter::Wire(tags.map(|tags| tags.split(';'))),
                TagStorage::Owned(tags) => TagIter::Owned(tags.iter()),
            },
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
            inner: match self.params {
                ParamStorage::Wire(params) => ParamIter::Wire(params),
                ParamStorage::Owned(params) => ParamIter::Owned(params.iter()),
            },
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

    /// Copies this borrowed view into an owned message without re-parsing it.
    #[must_use]
    pub fn to_owned(&self) -> Message {
        let mut message = Message::new(self.command, self.params());
        message.source = self.source.map(str::to_owned);
        for (key, value) in self.tags() {
            message.insert_tag(key.to_owned(), value.map(Cow::into_owned));
        }
        message
    }
}

/// An owned IRC message that can be validated and serialized for the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    tags: Vec<(String, Option<String>)>,
    source: Option<String>,
    command: String,
    params: Vec<String>,
}

impl Message {
    /// Creates a message from a command and its parameters.
    pub fn new<C, P, S>(command: C, params: P) -> Self
    where
        C: Into<String>,
        P: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            tags: Vec::new(),
            source: None,
            command: command.into(),
            params: params.into_iter().map(Into::into).collect(),
        }
    }

    /// Creates a `PRIVMSG` for a target.
    pub fn privmsg(target: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new("PRIVMSG", [target.into(), text.into()])
    }

    /// Creates a `NOTICE` for a target.
    pub fn notice(target: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new("NOTICE", [target.into(), text.into()])
    }

    /// Creates a `TAGMSG` for a target.
    pub fn tagmsg(target: impl Into<String>) -> Self {
        Self::new("TAGMSG", [target.into()])
    }

    /// Creates a `JOIN` for a channel or comma-separated channel list.
    pub fn join(channels: impl Into<String>) -> Self {
        Self::new("JOIN", [channels.into()])
    }

    /// Creates a `PART` without a reason.
    pub fn part(channels: impl Into<String>) -> Self {
        Self::new("PART", [channels.into()])
    }

    /// Creates a `PART` with a reason.
    pub fn part_with_reason(channels: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new("PART", [channels.into(), reason.into()])
    }

    /// Creates a message for an arbitrary command.
    pub fn raw<C, P, S>(command: C, params: P) -> Self
    where
        C: Into<String>,
        P: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(command, params)
    }

    /// Adds or replaces a tag, retaining its first insertion position.
    #[must_use]
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.insert_tag(key.into(), Some(value.into()));
        self
    }

    /// Sets the optional message source.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Borrows this message as a [`MessageRef`] without re-parsing it.
    #[must_use]
    pub fn as_ref(&self) -> MessageRef<'_> {
        MessageRef {
            tags: TagStorage::Owned(&self.tags),
            source: self.source.as_deref(),
            command: &self.command,
            params: ParamStorage::Owned(&self.params),
        }
    }

    fn insert_tag(&mut self, key: String, value: Option<String>) {
        if let Some((_, stored)) = self.tags.iter_mut().find(|(stored, _)| stored == &key) {
            *stored = value;
        } else {
            self.tags.push((key, value));
        }
    }
}

/// An iterator over the tags of a borrowed IRC message.
#[derive(Clone, Debug)]
pub struct Tags<'a> {
    inner: TagIter<'a>,
}

#[derive(Clone, Debug)]
enum TagIter<'a> {
    Wire(Option<std::str::Split<'a, char>>),
    Owned(slice::Iter<'a, (String, Option<String>)>),
}

impl<'a> Iterator for Tags<'a> {
    type Item = (&'a str, Option<Cow<'a, str>>);

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            TagIter::Wire(tags) => {
                let tag = tags.as_mut()?.next()?;
                let (key, value) = tag.split_once('=').map_or((tag, None), |(key, value)| {
                    (key, (!value.is_empty()).then_some(value))
                });
                Some((key, value.map(unescape_tag)))
            }
            TagIter::Owned(tags) => tags
                .next()
                .map(|(key, value)| (key.as_str(), value.as_deref().map(Cow::Borrowed))),
        }
    }
}

impl FusedIterator for Tags<'_> {}

/// An iterator over the parameters of a borrowed IRC message.
#[derive(Clone, Debug)]
pub struct Params<'a> {
    inner: ParamIter<'a>,
}

#[derive(Clone, Debug)]
enum ParamIter<'a> {
    Wire(&'a str),
    Owned(slice::Iter<'a, String>),
}

impl<'a> Iterator for Params<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            ParamIter::Wire(remaining) => {
                *remaining = remaining.trim_start_matches(' ');
                if remaining.is_empty() {
                    return None;
                }
                if let Some(trailing) = remaining.strip_prefix(':') {
                    *remaining = "";
                    return Some(trailing);
                }
                let (param, rest) = remaining.split_once(' ').unwrap_or((remaining, ""));
                *remaining = rest;
                Some(param)
            }
            ParamIter::Owned(params) => params.next().map(String::as_str),
        }
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

#[allow(dead_code)]
fn escape_tag(value: &str) -> Cow<'_, str> {
    if !value
        .chars()
        .any(|character| matches!(character, ';' | ' ' | '\\' | '\r' | '\n'))
    {
        return Cow::Borrowed(value);
    }

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            ';' => escaped.push_str(r"\:"),
            ' ' => escaped.push_str(r"\s"),
            '\\' => escaped.push_str(r"\\"),
            '\r' => escaped.push_str(r"\r"),
            '\n' => escaped.push_str(r"\n"),
            other => escaped.push(other),
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{escape_tag, unescape_tag};

    proptest! {
        #[test]
        fn escaping_round_trips(value in any::<String>()) {
            let escaped = escape_tag(&value);
            prop_assert_eq!(unescape_tag(&escaped), value.as_str());
        }
    }

    #[test]
    fn escapes_each_special_character() {
        assert_eq!(
            escape_tag("semi; space \\ cr\r lf\n"),
            r"semi\:\sspace\s\\\scr\r\slf\n"
        );
    }
}
