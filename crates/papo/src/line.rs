use std::{fmt, str::FromStr};

use crate::{BuildError, LineSource, Param, WireLine};

/// An owned raw IRC line. Parameters parsed from text are ordinary text.
/// Pass a [`crate::Secret`] when building a line to enable redaction and buffer wiping.
#[derive(Clone, Eq, PartialEq)]
pub struct Line(pub(crate) papo_proto::Line);

macro_rules! constructors {
    ($($(#[$doc:meta])* fn $name:ident($($arg:ident),*);)*) => {
        $(
            $(#[$doc])*
            pub fn $name($($arg: impl Into<String>),*) -> Self {
                Self(papo_proto::Line::$name($($arg),*))
            }
        )*
    };
}

macro_rules! accessors {
    ($($(#[$doc:meta])* fn $name:ident($($arg:ident: $ty:ty),*) -> $result:ty;)*) => {
        $(
            $(#[$doc])*
            #[must_use]
            pub fn $name(&self, $($arg: $ty),*) -> $result {
                self.0.$name($($arg),*)
            }
        )*
    };
}

impl Line {
    constructors! {
        /// Starts a line with no parameters.
        fn new(command);
        /// Creates a raw `PRIVMSG` without checking who holds the target nick.
        fn privmsg(target, text);
        /// Creates a raw `NOTICE` without checking who holds the target nick.
        fn notice(target, text);
        /// Creates a raw `TAGMSG` without checking who holds the target nick.
        fn tagmsg(target);
        /// Creates a `JOIN` for a channel or comma-separated channel list.
        fn join(channels);
        /// Creates a `PART` without a reason.
        fn part(channels);
        /// Creates a `PART` with a reason.
        fn part_with_reason(channels, reason);
    }

    accessors! {
        /// Returns the command with its original spelling.
        fn command() -> &str;
        /// Returns the command as a three-digit numeric reply or error.
        fn numeric() -> Option<u16>;
        /// Returns the parameters in wire order.
        fn params() -> &[Param];
        /// Returns the parameter at the given index in wire order.
        fn get(index: usize) -> Option<&str>;
        /// Returns the last value for this tag, with an empty string for a valueless tag.
        fn tag(key: &str) -> Option<&str>;
        /// Returns tags in wire order, including repeated keys.
        fn tags() -> impl ExactSizeIterator<Item = (&str, &str)>;
        /// Returns the parsed source, when supplied by the server.
        fn source() -> Option<LineSource<'_>>;
    }

    /// Appends one parameter to the line.
    #[must_use]
    pub fn param(self, param: impl Into<Param>) -> Self {
        Self(self.0.param(param))
    }

    /// Appends parameters from an iterator in order.
    #[must_use]
    pub fn with_params(self, params: impl IntoIterator<Item = impl Into<Param>>) -> Self {
        Self(self.0.with_params(params))
    }

    /// Sets a tag, normalizing an empty value to a valueless tag.
    #[must_use]
    pub fn with_tag(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        Self(self.0.with_tag(key, value))
    }

    /// Sets the source on a server-originated line. IRC clients must omit the source.
    #[must_use]
    pub fn with_source(self, source: impl Into<String>) -> Self {
        Self(self.0.with_source(source))
    }

    pub(crate) fn as_ref(&self) -> papo_proto::LineRef<'_> {
        self.0.as_ref()
    }

    /// Encodes the line with CRLF, retaining secret provenance for redaction and wiping.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] for fields or lengths that cannot be represented on the wire.
    pub fn encode(&self) -> Result<WireLine, BuildError> {
        self.0.encode()
    }
}

impl FromStr for Line {
    type Err = <papo_proto::Line as FromStr>::Err;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

impl fmt::Display for Line {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl fmt::Debug for Line {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}
