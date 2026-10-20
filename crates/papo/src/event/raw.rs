use std::fmt;

use papo_proto::Message;

use super::{Source, Tags};

/// An owned IRC line not represented by a typed event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawLine {
    command: String,
    params: Vec<String>,
    tags: Tags,
    source: Option<Source>,
}

impl RawLine {
    /// Returns the command with its original spelling.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
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

    /// Returns the command as a three-digit numeric reply or error.
    #[must_use]
    pub fn numeric(&self) -> Option<u16> {
        let papo_proto::Command::Numeric(numeric) = papo_proto::Command::parse(&self.command)
        else {
            return None;
        };
        Some(numeric.code())
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
    pub(crate) fn from_proto(message: &Message) -> Self {
        let message = message.as_ref();
        Self {
            command: message.raw_command().to_owned(),
            params: message.params().map(str::to_owned).collect(),
            tags: Tags::from_message(&message),
            source: message.source().map(Source::from_proto),
        }
    }
}

impl fmt::Display for RawLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.tags.is_empty() {
            formatter.write_str("@")?;
            for (index, (key, value)) in self.tags.iter().enumerate() {
                if index != 0 {
                    formatter.write_str(";")?;
                }
                formatter.write_str(key)?;
                write!(formatter, "={}", escape_tag(value))?;
            }
            formatter.write_str(" ")?;
        }
        if let Some(source) = &self.source {
            formatter.write_str(":")?;
            match source {
                Source::User { nick, user, host } => {
                    formatter.write_str(nick)?;
                    if let Some(user) = user {
                        write!(formatter, "!{user}")?;
                    }
                    if let Some(host) = host {
                        write!(formatter, "@{host}")?;
                    }
                }
                Source::Server { name } => formatter.write_str(name)?,
            }
            formatter.write_str(" ")?;
        }
        formatter.write_str(&self.command)?;
        for (index, param) in self.params.iter().enumerate() {
            formatter.write_str(" ")?;
            if index + 1 == self.params.len()
                && (param.is_empty() || param.contains(' ') || param.starts_with(':'))
            {
                formatter.write_str(":")?;
            }
            formatter.write_str(param)?;
        }
        Ok(())
    }
}

fn escape_tag(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\:")
        .replace(' ', "\\s")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use papo_proto::LineRef;

    use super::{RawLine, Source};

    #[test]
    fn owns_and_formats_every_wire_component() {
        let message = LineRef::parse(
            "@account=alice;example.org/note=one\\stwo\\:three :alice!~a@cloak.example NOTICE #papo :maintenance soon",
        )
        .unwrap()
        .to_owned();
        let line = RawLine::from_proto(&message);

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
        let line = RawLine::from_proto(&message);

        assert_eq!(line.tag("bot"), Some(""));
        assert_eq!(line.params(), ["papo", ""]);
        assert_eq!(line.param(1), Some(""));
        assert_eq!(line.numeric(), Some(372));
        assert!(matches!(
            line.source(),
            Some(Source::Server { name }) if name == "ergo.test"
        ));
        assert_eq!(line.source().unwrap().to_string(), "ergo.test");
        assert_eq!(line.to_string(), "@bot= :ergo.test 372 papo :");

        let mode = LineRef::parse(":papo-driver!papo@localhost MODE #papo +o alice")
            .unwrap()
            .to_owned();
        assert_eq!(
            RawLine::from_proto(&mode).to_string(),
            ":papo-driver!papo@localhost MODE #papo +o alice"
        );
    }
}
