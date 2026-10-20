use crate::{Command, LineRef, Params};

/// Severity of an `IRCv3` standard reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StandardReplyKind {
    /// The requested operation failed.
    Fail,
    /// The requested operation completed with a warning.
    Warn,
    /// The server supplied informational context.
    Note,
}

/// A typed view of an `IRCv3` standard reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StandardReply<'a> {
    /// Reply severity.
    pub kind: StandardReplyKind,
    /// Command associated with the reply, or `*` when unspecified.
    pub command: &'a str,
    /// Case-insensitive reply code in its original spelling.
    pub code: &'a str,
    /// Additional parameters supplied for diagnostics.
    pub context: Vec<&'a str>,
    /// Human-readable reply description.
    pub description: &'a str,
}

impl<'a> StandardReply<'a> {
    /// Parses a `FAIL`, `WARN`, or `NOTE` with at least three parameters.
    #[must_use]
    pub fn parse(line: &LineRef<'a>) -> Option<Self> {
        let kind = match line.command() {
            Command::Fail => StandardReplyKind::Fail,
            Command::Warn => StandardReplyKind::Warn,
            Command::Note => StandardReplyKind::Note,
            _ => return None,
        };
        let mut params: Vec<_> = line.params().collect();
        if params.len() < 3 {
            return None;
        }
        let description = params.pop()?;
        let command = params.remove(0);
        let code = params.remove(0);
        Some(Self {
            kind,
            command,
            code,
            context: params,
            description,
        })
    }
}

/// A typed view of a plain or extended `JOIN` line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Join<'a> {
    /// The joined channel.
    pub channel: &'a str,
    /// The user's account from extended-join, excluding the logged-out marker.
    pub account: Option<&'a str>,
    /// The user's real name when extended-join is active.
    pub realname: Option<&'a str>,
}

impl<'a> Join<'a> {
    /// Parses a `JOIN` with either one plain parameter or three extended parameters.
    #[must_use]
    pub fn parse(line: &LineRef<'a>) -> Option<Self> {
        if line.command() != Command::Join {
            return None;
        }
        let mut params = line.params();
        let channel = params.next()?;
        let account = params.next();
        let realname = params.next();
        if params.next().is_some() {
            return None;
        }
        match (account, realname) {
            (None, None) => Some(Self {
                channel,
                account: None,
                realname: None,
            }),
            (Some(account), Some(realname)) => Some(Self {
                channel,
                account: (account != "*").then_some(account),
                realname: Some(realname),
            }),
            _ => None,
        }
    }
}

/// A typed view of an account-notify `ACCOUNT` line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Account<'a> {
    /// The new account name, or `None` when the user logged out.
    pub account: Option<&'a str>,
}

impl<'a> Account<'a> {
    /// Parses an `ACCOUNT` line with exactly one parameter.
    #[must_use]
    pub fn parse(line: &LineRef<'a>) -> Option<Self> {
        let mut params = exact_params(line, Command::Account, 1)?;
        let account = params.next()?;
        Some(Self {
            account: (account != "*").then_some(account),
        })
    }
}

/// A typed view of an away-notify `AWAY` line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Away<'a> {
    /// The away message, or `None` when the user returned.
    pub message: Option<&'a str>,
}

impl<'a> Away<'a> {
    /// Parses an `AWAY` line with zero or one parameter.
    #[must_use]
    pub fn parse(line: &LineRef<'a>) -> Option<Self> {
        if line.command() != Command::Away {
            return None;
        }
        let mut params = line.params();
        let message = params.next();
        params.next().is_none().then_some(Self { message })
    }
}

/// A typed view of a `CHGHOST` line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Chghost<'a> {
    /// The user's new username, including a possible `~` prefix.
    pub user: &'a str,
    /// The user's new hostname.
    pub host: &'a str,
}

impl<'a> Chghost<'a> {
    /// Parses a `CHGHOST` line with exactly two parameters.
    #[must_use]
    pub fn parse(line: &LineRef<'a>) -> Option<Self> {
        let mut params = exact_params(line, Command::Chghost, 2)?;
        Some(Self {
            user: params.next()?,
            host: params.next()?,
        })
    }
}

/// A typed view of a `SETNAME` line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Setname<'a> {
    /// The user's new real name.
    pub realname: &'a str,
}

impl<'a> Setname<'a> {
    /// Parses a `SETNAME` line with exactly one parameter.
    #[must_use]
    pub fn parse(line: &LineRef<'a>) -> Option<Self> {
        let mut params = exact_params(line, Command::Setname, 1)?;
        Some(Self {
            realname: params.next()?,
        })
    }
}

fn exact_params<'a>(line: &LineRef<'a>, command: Command<'a>, length: usize) -> Option<Params<'a>> {
    (line.command() == command && line.len() == length).then(|| line.params())
}
