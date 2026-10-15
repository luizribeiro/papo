use crate::{Command, MessageRef};

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
