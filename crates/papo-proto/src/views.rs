use crate::{Command, MessageRef, Params};

/// A typed view of a plain or extended `JOIN` message.
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
    pub fn parse(message: &MessageRef<'a>) -> Option<Self> {
        if message.command() != Command::Join {
            return None;
        }
        let mut params = message.params();
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

/// A typed view of an account-notify `ACCOUNT` message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Account<'a> {
    /// The new account name, or `None` when the user logged out.
    pub account: Option<&'a str>,
}

impl<'a> Account<'a> {
    /// Parses an `ACCOUNT` message with exactly one parameter.
    #[must_use]
    pub fn parse(message: &MessageRef<'a>) -> Option<Self> {
        let mut params = exact_params(message, Command::Account, 1)?;
        let account = params.next()?;
        Some(Self {
            account: (account != "*").then_some(account),
        })
    }
}

/// A typed view of an away-notify `AWAY` message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Away<'a> {
    /// The away message, or `None` when the user returned.
    pub message: Option<&'a str>,
}

impl<'a> Away<'a> {
    /// Parses an `AWAY` message with zero or one parameter.
    #[must_use]
    pub fn parse(message: &MessageRef<'a>) -> Option<Self> {
        if message.command() != Command::Away {
            return None;
        }
        let mut params = message.params();
        let message = params.next();
        params.next().is_none().then_some(Self { message })
    }
}

fn exact_params<'a>(
    message: &MessageRef<'a>,
    command: Command<'a>,
    length: usize,
) -> Option<Params<'a>> {
    (message.command() == command && message.len() == length).then(|| message.params())
}
