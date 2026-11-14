use std::collections::HashMap;

use papo_proto::{LineRef, Prefix};

use crate::{Conversation, RequestError, Session};

#[derive(Clone, Debug)]
struct Audience {
    channel: Conversation,
    status: Prefix,
}

/// The ids of recent status-prefixed lines and the audience each one reached.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpsOnlyLines {
    audiences: HashMap<String, Audience>,
}

/// Where a text or tag message goes.
fn line_target<'a>(message: &LineRef<'a>) -> Option<&'a str> {
    ["PRIVMSG", "NOTICE", "TAGMSG"]
        .iter()
        .any(|command| message.command_is(command))
        .then(|| message.param(0))
        .flatten()
}

/// The target and referenced id of a line that threads or reacts to a message.
fn reference<'a>(message: &LineRef<'a>) -> Option<(&'a str, String)> {
    let id = message
        .tag("+reply")
        .or_else(|| message.tag("+draft/reply"))?;
    Some((line_target(message)?, id.into_owned()))
}

impl Session {
    pub(crate) fn remember_ops_only(&mut self, message: &LineRef<'_>) {
        let (Some(msgid), Some(target)) = (message.tag("msgid"), line_target(message)) else {
            return;
        };
        if let Ok((Some(status), channel @ Conversation::Channel(_))) =
            self.state.parse_target(target)
        {
            self.ops_only
                .audiences
                .insert(msgid.into_owned(), Audience { channel, status });
        }
    }

    pub(crate) fn check_line_audience(&self, message: &LineRef<'_>) -> Result<(), RequestError> {
        reference(message).map_or(Ok(()), |(target, id)| self.check_audience(target, &id))
    }

    /// Refuses a send to `target` that points at a remembered ops-only line it would widen.
    pub(crate) fn check_audience(&self, target: &str, msgid: &str) -> Result<(), RequestError> {
        let Some(original) = self.ops_only.audiences.get(msgid) else {
            return Ok(());
        };
        let narrower = match self.state.parse_target(target) {
            Ok((Some(status), channel)) => {
                channel == original.channel && status.symbol() == original.status.symbol()
            }
            _ => false,
        };
        if narrower {
            Ok(())
        } else {
            Err(RequestError::WiderThanReferenced {
                referenced: original.status,
            })
        }
    }
}
