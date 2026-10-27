use std::{
    borrow::Cow,
    error::Error,
    fmt,
    iter::FusedIterator,
    ops::{Deref, Range},
    slice,
    str::{FromStr, Utf8Error},
};
use zeroize::Zeroize;

use crate::{Command, Secret, Source};

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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    Owned(&'a [Param]),
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

    /// Returns the parameter at `index`, if present.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&'a str> {
        self.param(index)
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
        let mut line = Line::new(self.command).with_params(self.params());
        line.source = self.source.map(str::to_owned);
        line.tags = self
            .tags()
            .map(|(key, value)| (key.to_owned(), value.map(Cow::into_owned)))
            .collect();
        line
    }
}

/// One owned IRC line parameter.
#[derive(Clone, Eq, PartialEq)]
pub struct Param(ParamValue);

#[derive(Clone, Eq, PartialEq)]
enum ParamValue {
    Text(String),
    Secret(Secret),
}

impl Param {
    fn as_str(&self) -> &str {
        match &self.0 {
            ParamValue::Text(value) => value,
            ParamValue::Secret(value) => value.expose(),
        }
    }

    fn is_secret(&self) -> bool {
        matches!(self.0, ParamValue::Secret(_))
    }
}

impl std::ops::Deref for Param {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Debug for Param {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ParamValue::Text(_) => self.as_str().fmt(formatter),
            ParamValue::Secret(_) => formatter.write_str("[REDACTED]"),
        }
    }
}

impl fmt::Display for Param {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ParamValue::Text(_) => formatter.write_str(self.as_str()),
            ParamValue::Secret(_) => formatter.write_str("<redacted>"),
        }
    }
}

impl<T: Into<String>> From<T> for Param {
    fn from(value: T) -> Self {
        Self(ParamValue::Text(value.into()))
    }
}

impl From<Secret> for Param {
    fn from(value: Secret) -> Self {
        Self(ParamValue::Secret(value))
    }
}

impl From<&Secret> for Param {
    fn from(value: &Secret) -> Self {
        Self(ParamValue::Secret(value.clone()))
    }
}

impl PartialEq<str> for Param {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Param {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// An encoded IRC line that retains secret provenance for safe formatting and disposal.
///
/// `Debug` and `Display` show ordinary line content, omit CRLF, and replace only parameters
/// originating from [`Secret`]. The owned byte buffer of a secret-bearing line is wiped on drop.
#[derive(Clone, Eq, PartialEq)]
pub struct WireLine {
    bytes: Vec<u8>,
    secret_spans: Vec<Range<usize>>,
    show_secret_lengths: bool,
}

impl WireLine {
    fn into_public_string(mut self) -> Option<String> {
        if !self.secret_spans.is_empty() {
            return None;
        }
        String::from_utf8(std::mem::take(&mut self.bytes)).ok()
    }

    fn fmt_redacted(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let visible_len = self.bytes.len().saturating_sub(2);
        let mut start = 0;
        for span in &self.secret_spans {
            formatter.write_str(&String::from_utf8_lossy(&self.bytes[start..span.start]))?;
            if self.show_secret_lengths {
                write!(formatter, "<redacted, {} bytes>", span.len())?;
            } else {
                formatter.write_str("<redacted>")?;
            }
            start = span.end;
        }
        formatter.write_str(&String::from_utf8_lossy(&self.bytes[start..visible_len]))
    }
}

impl Deref for WireLine {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

impl AsRef<[u8]> for WireLine {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl PartialEq<[u8]> for WireLine {
    fn eq(&self, other: &[u8]) -> bool {
        self.bytes == other
    }
}

impl fmt::Debug for WireLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_redacted(formatter)
    }
}

impl fmt::Display for WireLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_redacted(formatter)
    }
}

impl Drop for WireLine {
    fn drop(&mut self) {
        if !self.secret_spans.is_empty() {
            self.bytes.zeroize();
        }
    }
}

/// An owned IRC line that can be validated and serialized for the wire.
///
/// Parameters appended from [`Secret`] retain secret provenance for redacted formatting and
/// wiping. Parsing a complete line from text treats every parameter as ordinary text; callers that
/// put credentials in parsed raw lines intentionally give up those protections.
#[derive(Clone, Eq, PartialEq)]
pub struct Line {
    tags: Vec<(String, Option<String>)>,
    source: Option<String>,
    command: String,
    params: Vec<Param>,
}

impl Line {
    /// Starts a line with no parameters.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            tags: Vec::new(),
            source: None,
            command: command.into(),
            params: Vec::new(),
        }
    }

    /// Appends one parameter to the line.
    #[must_use]
    pub fn param(mut self, param: impl Into<Param>) -> Self {
        self.params.push(param.into());
        self
    }

    /// Appends parameters from an iterator in order.
    #[must_use]
    pub fn with_params(mut self, params: impl IntoIterator<Item = impl Into<Param>>) -> Self {
        self.params.extend(params.into_iter().map(Into::into));
        self
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
    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// Returns the parameter at `index` in wire order.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&str> {
        self.params.get(index).map(Param::as_str)
    }

    /// Marks a parameter as sensitive while preserving its text value.
    #[doc(hidden)]
    pub fn __mark_secret(&mut self, index: usize) {
        let Some(param) = self.params.get_mut(index) else {
            return;
        };
        if param.is_secret() {
            return;
        }
        let ParamValue::Text(value) =
            std::mem::replace(&mut param.0, ParamValue::Text(String::new()))
        else {
            unreachable!("secret parameters returned above");
        };
        param.0 = ParamValue::Secret(Secret::from(value));
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
        Self::new("PRIVMSG").param(target).param(text)
    }

    /// Creates a `NOTICE` for a target.
    pub fn notice(target: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new("NOTICE").param(target).param(text)
    }

    /// Creates a `TAGMSG` for a target.
    pub fn tagmsg(target: impl Into<String>) -> Self {
        Self::new("TAGMSG").param(target)
    }

    /// Creates a `JOIN` for a channel or comma-separated channel list.
    pub fn join(channels: impl Into<String>) -> Self {
        Self::new("JOIN").param(channels)
    }

    /// Creates a `PART` without a reason.
    pub fn part(channels: impl Into<String>) -> Self {
        Self::new("PART").param(channels)
    }

    /// Creates a `PART` with a reason.
    pub fn part_with_reason(channels: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new("PART").param(channels).param(reason)
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

    /// Serializes a line containing only ordinary parameters with its terminating CRLF.
    ///
    /// A colon is added to the final parameter only when required by the wire grammar.
    /// Use [`Self::encode`] for lines containing a [`Secret`], so the result retains the
    /// provenance needed for redaction and wiping.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] when a field cannot be represented safely or either wire length
    /// limit would be exceeded.
    pub fn to_line(&self) -> Result<String, BuildError> {
        self.encode()?
            .into_public_string()
            .ok_or(BuildError::InvalidParam(0))
    }

    /// Encodes this line while retaining secret provenance for redaction and wiping.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] when a field cannot be represented safely or either wire length
    /// limit would be exceeded.
    pub fn encode(&self) -> Result<WireLine, BuildError> {
        self.validate_fields()?;
        let params = self.params.iter().map(Param::as_str).collect::<Vec<_>>();
        let mut bytes = Vec::new();
        write_line(
            &self.tags,
            self.source.as_deref(),
            &self.command,
            &params,
            &mut bytes,
        )?;
        let mut offset = encoded_prefix_len(&self.tags, self.source.as_deref(), &self.command);
        let mut secret_spans = Vec::new();
        for (index, param) in self.params.iter().enumerate() {
            offset += 1 + usize::from(uses_trailing_prefix(&self.command, &params, index));
            if param.is_secret() {
                secret_spans.push(offset..offset + param.len());
            }
            offset += param.len();
        }
        Ok(WireLine {
            bytes,
            secret_spans,
            show_secret_lengths: self.command.eq_ignore_ascii_case("AUTHENTICATE"),
        })
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
    let tag_data_len = encoded_tag_data_len(tags);
    if tag_data_len > MAX_CLIENT_TAG_DATA_LEN {
        return Err(BuildError::TagsTooLong);
    }
    let body_len = source.map_or(0, |source| source.len() + 2)
        + command.len()
        + params.iter().enumerate().fold(0, |length, (index, param)| {
            length + 1 + usize::from(uses_trailing_prefix(command, params, index)) + param.len()
        })
        + 2;
    if body_len > MAX_LINE_LEN {
        return Err(BuildError::LineTooLong);
    }

    let tag_section_len = tags.first().map_or(0, |_| tag_data_len.saturating_add(2));
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
        if uses_trailing_prefix(command, params, index) {
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

fn encoded_tag_data_len(tags: &[(String, Option<String>)]) -> usize {
    let fields_len = tags
        .iter()
        .map(|(key, value)| {
            key.len().saturating_add(
                value
                    .as_deref()
                    .map_or(0, |value| escaped_tag_value_len(value).saturating_add(1)),
            )
        })
        .sum::<usize>();
    fields_len.saturating_add(tags.len().saturating_sub(1))
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
        self.encode().map_err(|_| fmt::Error)?.fmt(formatter)
    }
}

impl fmt::Debug for Line {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

fn needs_trailing_prefix(param: &str) -> bool {
    param.is_empty() || param.contains(' ') || param.starts_with(':')
}

fn uses_trailing_prefix(command: &str, params: &[&str], index: usize) -> bool {
    index + 1 == params.len()
        && (needs_trailing_prefix(params[index])
            || (command.eq_ignore_ascii_case("CAP")
                && params
                    .first()
                    .is_some_and(|param| param.eq_ignore_ascii_case("REQ"))))
}

fn encoded_prefix_len(
    tags: &[(String, Option<String>)],
    source: Option<&str>,
    command: &str,
) -> usize {
    let tag_len = tags
        .first()
        .map_or(0, |_| encoded_tag_data_len(tags).saturating_add(2));
    tag_len
        .saturating_add(source.map_or(0, |source| source.len().saturating_add(2)))
        .saturating_add(command.len())
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
    Owned(slice::Iter<'a, Param>),
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
            ParamIter::Owned(params) => params.next().map(Param::as_str),
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
