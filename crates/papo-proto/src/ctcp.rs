/// A parsed Client-to-Client Protocol message body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ctcp<'a> {
    /// The CTCP command with its original spelling.
    pub command: &'a str,
    /// The command parameters, when present.
    pub params: Option<&'a str>,
}

impl<'a> Ctcp<'a> {
    /// Parses the complete text parameter of a `PRIVMSG` or `NOTICE`.
    ///
    /// The closing delimiter is optional on input for compatibility with
    /// clients that split long `ACTION` messages incorrectly.
    #[must_use]
    pub fn parse(text: &'a str) -> Option<Self> {
        let body = text.strip_prefix('\x01')?;
        let body = body.strip_suffix('\x01').unwrap_or(body);
        let (command, params) = body
            .split_once(' ')
            .map_or((body, None), |(command, params)| (command, Some(params)));
        if !valid_command(command) || params.is_some_and(|params| !valid_params(params)) {
            return None;
        }
        Some(Self { command, params })
    }

    /// Builds a complete CTCP message body with a closing delimiter.
    ///
    /// # Panics
    ///
    /// Panics when `command` or `params` contains a byte forbidden by CTCP,
    /// or when either supplied field is empty.
    #[must_use]
    pub fn build(command: &str, params: Option<&str>) -> String {
        assert!(valid_command(command), "invalid CTCP command");
        assert!(params.is_none_or(valid_params), "invalid CTCP parameters");
        let mut message =
            String::with_capacity(command.len() + params.map_or(0, |params| params.len() + 1) + 2);
        message.push('\x01');
        message.push_str(command);
        if let Some(params) = params {
            message.push(' ');
            message.push_str(params);
        }
        message.push('\x01');
        message
    }

    /// Builds a CTCP `ACTION` message body.
    ///
    /// # Panics
    ///
    /// Panics when `text` is empty or contains a byte forbidden by CTCP.
    #[must_use]
    pub fn action(text: &str) -> String {
        Self::build("ACTION", Some(text))
    }
}

fn valid_command(command: &str) -> bool {
    !command.is_empty() && !command.contains(['\0', '\x01', '\r', '\n', ' '])
}

fn valid_params(params: &str) -> bool {
    !params.is_empty() && !params.contains(['\0', '\x01', '\r', '\n'])
}
