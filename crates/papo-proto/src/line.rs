use std::{
    borrow::Cow,
    error::Error,
    fmt,
    iter::FusedIterator,
    slice,
    str::{FromStr, Utf8Error},
};

use crate::{Command, Source};

/// Maximum byte length of an IRC line excluding its tag section but including CRLF.
///
/// This limit is defined by the [Modern IRC client protocol].
///
/// [Modern IRC client protocol]: https://modern.ircdocs.horse/#message-format
pub const MAX_LINE_LEN: usize = 512;

/// Maximum byte length of a received IRC tag section, including `@` and its trailing space.
///
/// This limit is defined by the [IRCv3 message-tags specification].
///
/// [IRCv3 message-tags specification]: https://ircv3.net/specs/extensions/message-tags#size-limit
pub const MAX_TAG_SECTION_LEN: usize = 8191;

/// Maximum byte length of tags supplied by a client, excluding `@` and the trailing space.
///
/// This limit is defined by the [IRCv3 message-tags specification].
///
/// [IRCv3 message-tags specification]: https://ircv3.net/specs/extensions/message-tags#size-limit
pub const MAX_CLIENT_TAG_DATA_LEN: usize = 4094;

/// A failure to parse an IRC line.
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ParseError {
    /// The line has no content before its line ending.
    EmptyLine,
    /// The line contains tags, a source, or spaces but no command.
    MissingCommand,
    /// The byte input is not valid UTF-8.
    InvalidUtf8,
    /// The input contains CR, LF, or NUL within the line.
    InvalidByte,
    /// The tag section or the rest of the line exceeds its wire limit.
    TooLong,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyLine => "IRC line is empty",
            Self::MissingCommand => "IRC line has no command",
            Self::InvalidUtf8 => "IRC line is not valid UTF-8",
            Self::InvalidByte => "IRC line contains CR, LF, or NUL",
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

/// A failure to build a valid IRC wire line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuildError {
    /// The command is empty or contains a wire delimiter.
    InvalidCommand,
    /// A parameter contains a forbidden byte or requires trailing position.
    InvalidParam(usize),
    /// The source is empty or contains a wire delimiter.
    InvalidSource,
    /// A tag key is empty or contains a wire delimiter.
    InvalidTagKey,
    /// A tag value contains NUL.
    InvalidTagValue,
    /// Client-supplied tag data exceeds its wire limit.
    TagsTooLong,
    /// The line body exceeds its wire limit.
    LineTooLong,
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand => formatter.write_str("IRC command is invalid"),
            Self::InvalidParam(index) => write!(formatter, "IRC parameter {index} is invalid"),
            Self::InvalidSource => formatter.write_str("IRC source is invalid"),
            Self::InvalidTagKey => formatter.write_str("IRC tag key is invalid"),
            Self::InvalidTagValue => formatter.write_str("IRC tag value is invalid"),
            Self::TagsTooLong => formatter.write_str("IRC client tag data exceeds its wire limit"),
            Self::LineTooLong => formatter.write_str("IRC line body exceeds its wire limit"),
        }
    }
}

impl Error for BuildError {}

/// A zero-copy view of one IRC line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineRef<'a> {
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

impl<'a> LineRef<'a> {
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

    /// Returns the line tags in wire order.
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

    /// Returns the parsed line source, when present.
    ///
    /// Nicknames never contain `.`, so a bare name containing `.` and no `!` or `@` is
    /// classified as a server name. All other sources are classified as users.
    #[must_use]
    pub fn source(&self) -> Option<Source<'a>> {
        self.source.map(Source::parse)
    }

    /// Returns the source without its leading colon, when present.
    #[must_use]
    pub const fn raw_source(&self) -> Option<&'a str> {
        self.source
    }

    /// Returns the parsed command.
    #[must_use]
    pub fn command(&self) -> Command<'a> {
        Command::parse(self.command)
    }

    /// Returns the command with its original spelling.
    #[must_use]
    pub const fn raw_command(&self) -> &'a str {
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

    /// Reports whether the line has no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.params().next().is_none()
    }

    /// Copies this borrowed view into an owned line without re-parsing it.
    #[must_use]
    pub fn to_owned(&self) -> Line {
        let mut line = Line::new(self.command, self.params());
        line.source = self.source.map(str::to_owned);
        line.tags = self
            .tags()
            .map(|(key, value)| (key.to_owned(), value.map(Cow::into_owned)))
            .collect();
        line
    }
}

/// An owned IRC line that can be validated and serialized for the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    tags: Vec<(String, Option<String>)>,
    source: Option<String>,
    command: String,
    params: Vec<String>,
}

impl Line {
    /// Creates a line from a command and its parameters.
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

    /// Returns the command with its original spelling.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Returns the command as a three-digit numeric reply or error.
    #[must_use]
    pub fn numeric(&self) -> Option<u16> {
        let Command::Numeric(numeric) = Command::parse(&self.command) else {
            return None;
        };
        Some(numeric.code())
    }

    /// Returns the parameters in wire order.
    #[must_use]
    pub fn params(&self) -> &[String] {
        &self.params
    }

    /// Returns the parameter at `index` in wire order.
    #[must_use]
    pub fn param(&self, index: usize) -> Option<&str> {
        self.params.get(index).map(String::as_str)
    }

    /// Returns the last value for a case-sensitive tag key.
    ///
    /// A tag without a value returns `Some("")`. `None` means the key is absent.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_deref().unwrap_or(""))
    }

    /// Iterates over tag keys and values in wire order.
    ///
    /// A tag without a value has an empty value. Duplicate keys remain visible.
    #[must_use]
    pub fn tags(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.tags
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_deref().unwrap_or("")))
    }

    /// Returns the parsed source, when supplied by the server.
    #[must_use]
    pub fn source(&self) -> Option<Source<'_>> {
        self.source.as_deref().map(Source::parse)
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

    /// Adds or replaces a tag, retaining its first insertion position.
    ///
    /// An empty value is normalized to a valueless tag.
    #[must_use]
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.insert_tag(key.into(), Some(value.into()));
        self
    }

    /// Sets the optional line source for tests and server-side use.
    ///
    /// IRC clients must not send a source.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Borrows this line as a [`LineRef`] without re-parsing it.
    #[must_use]
    pub fn as_ref(&self) -> LineRef<'_> {
        LineRef {
            tags: TagStorage::Owned(&self.tags),
            source: self.source.as_deref(),
            command: &self.command,
            params: ParamStorage::Owned(&self.params),
        }
    }

    /// Serializes this line with its terminating CRLF.
    ///
    /// A colon is added to the final parameter only when required by the wire grammar.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] when a field cannot be represented safely or either wire length
    /// limit would be exceeded.
    pub fn to_line(&self) -> Result<String, BuildError> {
        self.validate_fields()?;
        let params = self.params.iter().map(String::as_str).collect::<Vec<_>>();
        let mut bytes = Vec::new();
        write_line(
            &self.tags,
            self.source.as_deref(),
            &self.command,
            &params,
            &mut bytes,
        )?;
        String::from_utf8(bytes).map_err(|_| BuildError::InvalidParam(0))
    }

    fn validate_fields(&self) -> Result<(), BuildError> {
        if self.command.is_empty()
            || self.command.starts_with(['@', ':'])
            || self.command.contains([' ', '\r', '\n', '\0'])
        {
            return Err(BuildError::InvalidCommand);
        }
        if self
            .source
            .as_deref()
            .is_some_and(|source| source.is_empty() || source.contains([' ', '\r', '\n', '\0']))
        {
            return Err(BuildError::InvalidSource);
        }
        for (key, value) in &self.tags {
            if key.is_empty() || key.contains([' ', ';', '=', '\r', '\n', '\0']) {
                return Err(BuildError::InvalidTagKey);
            }
            if value.as_deref().is_some_and(|value| value.contains('\0')) {
                return Err(BuildError::InvalidTagValue);
            }
        }
        for (index, param) in self.params.iter().enumerate() {
            if param.contains(['\r', '\n', '\0'])
                || (index + 1 != self.params.len() && needs_trailing_prefix(param))
            {
                return Err(BuildError::InvalidParam(index));
            }
        }
        Ok(())
    }

    fn insert_tag(&mut self, key: String, value: Option<String>) {
        let value = value.filter(|value| !value.is_empty());
        if let Some((_, stored)) = self.tags.iter_mut().find(|(stored, _)| stored == &key) {
            *stored = value;
        } else {
            self.tags.push((key, value));
        }
    }
}

fn write_line(
    tags: &[(String, Option<String>)],
    source: Option<&str>,
    command: &str,
    params: &[&str],
    out: &mut Vec<u8>,
) -> Result<(), BuildError> {
    let tag_data_len = tags
        .iter()
        .enumerate()
        .fold(0, |length, (index, (key, value))| {
            length
                + usize::from(index != 0)
                + key.len()
                + value
                    .as_deref()
                    .map_or(0, |value| 1 + escaped_tag_value_len(value))
        });
    if tag_data_len > MAX_CLIENT_TAG_DATA_LEN {
        return Err(BuildError::TagsTooLong);
    }
    let body_len = source.map_or(0, |source| source.len() + 2)
        + command.len()
        + params.iter().enumerate().fold(0, |length, (index, param)| {
            length
                + 1
                + usize::from(index + 1 == params.len() && needs_trailing_prefix(param))
                + param.len()
        })
        + 2;
    if body_len > MAX_LINE_LEN {
        return Err(BuildError::LineTooLong);
    }

    let tag_section_len = usize::from(!tags.is_empty()) * (tag_data_len + 2);
    out.reserve_exact(tag_section_len + body_len);
    if !tags.is_empty() {
        out.push(b'@');
        for (index, (key, value)) in tags.iter().enumerate() {
            if index != 0 {
                out.push(b';');
            }
            out.extend_from_slice(key.as_bytes());
            if let Some(value) = value {
                out.push(b'=');
                out.extend_from_slice(escape_tag_value(value).as_bytes());
            }
        }
        out.push(b' ');
    }
    if let Some(source) = source {
        out.push(b':');
        out.extend_from_slice(source.as_bytes());
        out.push(b' ');
    }
    out.extend_from_slice(command.as_bytes());
    for (index, param) in params.iter().enumerate() {
        out.push(b' ');
        if index + 1 == params.len() && needs_trailing_prefix(param) {
            out.push(b':');
        }
        out.extend_from_slice(param.as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    Ok(())
}

fn escaped_tag_value_len(value: &str) -> usize {
    value
        .chars()
        .map(|character| match character {
            ';' | ' ' | '\\' | '\r' | '\n' => 2,
            other => other.len_utf8(),
        })
        .sum()
}

impl FromStr for Line {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.contains(['\r', '\n', '\0']) {
            return Err(ParseError::InvalidByte);
        }
        LineRef::parse(input).map(|line| line.to_owned())
    }
}

impl fmt::Display for Line {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wire = self.to_line().map_err(|_| fmt::Error)?;
        formatter.write_str(wire.strip_suffix("\r\n").unwrap_or(&wire))
    }
}

fn needs_trailing_prefix(param: &str) -> bool {
    param.is_empty() || param.contains(' ') || param.starts_with(':')
}

/// An iterator over the tags of a borrowed IRC line.
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
                    let value = unescape_tag(value);
                    (key, (!value.is_empty()).then_some(value))
                });
                Some((key, value))
            }
            TagIter::Owned(tags) => tags
                .next()
                .map(|(key, value)| (key.as_str(), value.as_deref().map(Cow::Borrowed))),
        }
    }
}

impl FusedIterator for Tags<'_> {}

/// An iterator over the parameters of a borrowed IRC line.
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
    let line_len = line.len() - tag_section_len + 2;
    if tag_section_len > MAX_TAG_SECTION_LEN || line_len > MAX_LINE_LEN {
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

/// Escapes an `IRCv3` tag value for use on the wire.
#[must_use]
pub fn escape_tag_value(value: &str) -> Cow<'_, str> {
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

    use super::{escape_tag_value, unescape_tag};

    proptest! {
        #[test]
        fn escaping_round_trips(value in any::<String>()) {
            let escaped = escape_tag_value(&value);
            prop_assert_eq!(unescape_tag(&escaped), value.as_str());
        }
    }

    #[test]
    fn escapes_each_special_character() {
        assert_eq!(
            escape_tag_value("semi; space \\ cr\r lf\n"),
            r"semi\:\sspace\s\\\scr\r\slf\n"
        );
    }
}
