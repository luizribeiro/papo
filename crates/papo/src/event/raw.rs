use std::fmt;

use papo_proto::{Casemapping, Line};

use super::{Source, Tags};

/// An owned IRC line not represented by a typed event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawLine {
    line: Line,
    tags: Tags,
    source: Option<Source>,
}

impl RawLine {
    /// Returns the command with its original spelling.
    #[must_use]
    pub fn command(&self) -> &str {
        self.line.command()
    }

    /// Returns the parameters in wire order.
    #[must_use]
    pub fn params(&self) -> &[String] {
        self.line.params()
    }

    /// Returns the parameter at `index` in wire order.
    #[must_use]
    pub fn param(&self, index: usize) -> Option<&str> {
        self.line.param(index)
    }

    /// Returns the command as a three-digit numeric reply or error.
    #[must_use]
    pub fn numeric(&self) -> Option<u16> {
        self.line.numeric()
    }

    /// Returns the last value for a case-sensitive tag key.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key)
    }

    /// Returns the message tags.
    #[must_use]
    pub const fn tags(&self) -> &Tags {
        &self.tags
    }

    /// Returns the parsed source, when supplied by the server.
    #[must_use]
    pub fn source(&self) -> Option<Source> {
        self.source.clone()
    }
}

impl RawLine {
    pub(crate) fn from_proto(message: &Line, casemapping: Casemapping) -> Self {
        Self {
            line: message.clone(),
            tags: Tags::from_message(&message.as_ref()),
            source: message
                .as_ref()
                .source()
                .map(|source| Source::from_proto(source, casemapping)),
        }
    }
}

impl fmt::Display for RawLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.line.fmt(formatter)
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use papo_proto::{Casemapping, LineRef};

    use super::{RawLine, Source};

    #[test]
    fn owns_and_formats_every_wire_component() {
        let message = LineRef::parse(
            "@account=alice;example.org/note=one\\stwo\\:three :alice!~a@cloak.example NOTICE #papo :maintenance soon",
        )
        .unwrap()
        .to_owned();
        let line = RawLine::from_proto(&message, Casemapping::default());

        assert_eq!(line.command(), "NOTICE");
        assert_eq!(line.params(), ["#papo", "maintenance soon"]);
        assert_eq!(line.param(0), Some("#papo"));
        assert_eq!(line.param(1), Some("maintenance soon"));
        assert_eq!(line.param(2), None);
        assert_eq!(line.numeric(), None);
        assert_eq!(line.tag("account"), Some("alice"));
        assert_eq!(line.tag("example.org/note"), Some("one two;three"));
        assert!(matches!(
            line.source(),
            Some(Source::User { nick, user: Some(user), host: Some(host) })
                if nick == "alice" && user == "~a" && host == "cloak.example"
        ));
        assert_eq!(line.source().unwrap().to_string(), "alice");
        assert_eq!(
            line.to_string(),
            "@account=alice;example.org/note=one\\stwo\\:three :alice!~a@cloak.example NOTICE #papo :maintenance soon"
        );
    }

    #[test]
    fn preserves_bare_tags_and_empty_trailing_parameters() {
        let message = LineRef::parse("@bot :ergo.test 372 papo :")
            .unwrap()
            .to_owned();
        let line = RawLine::from_proto(&message, Casemapping::default());

        assert_eq!(line.tag("bot"), Some(""));
        assert_eq!(line.params(), ["papo", ""]);
        assert_eq!(line.param(1), Some(""));
        assert_eq!(line.numeric(), Some(372));
        assert!(matches!(
            line.source(),
            Some(Source::Server { name }) if name == "ergo.test"
        ));
        assert_eq!(line.source().unwrap().to_string(), "ergo.test");
        assert_eq!(line.to_string(), "@bot :ergo.test 372 papo :");

        let mode = LineRef::parse(":papo-driver!papo@localhost MODE #papo +o alice")
            .unwrap()
            .to_owned();
        assert_eq!(
            RawLine::from_proto(&mode, Casemapping::default()).to_string(),
            ":papo-driver!papo@localhost MODE #papo +o alice"
        );
    }

    #[test]
    fn source_nicks_capture_the_supplied_casemapping() {
        let message = LineRef::parse(":Alice[bot]!~a@cloak.example NOTICE * :maintenance")
            .unwrap()
            .to_owned();
        let line = RawLine::from_proto(&message, Casemapping::Ascii);
        let Some(Source::User { nick, .. }) = line.source() else {
            panic!("line should have a user source");
        };

        assert_eq!(nick.casemapping(), Casemapping::Ascii);
        assert_ne!(nick, "alice{bot}");
    }
}
