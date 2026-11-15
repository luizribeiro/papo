use std::collections::{HashMap, VecDeque};

use papo_proto::{Isupport, LineRef, Prefix};

use crate::{Conversation, RequestError, Session};

/// How many ops-only message ids one connection remembers.
const REMEMBERED: usize = 1024;

#[derive(Clone, Debug)]
struct Audience {
    channel: Conversation,
    status: Prefix,
}

/// The ids of recent status-prefixed lines and the audience each one reached.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpsOnlyLines {
    order: VecDeque<String>,
    audiences: HashMap<String, Audience>,
}

impl OpsOnlyLines {
    fn remember(&mut self, msgid: String, audience: Audience) {
        if self.audiences.insert(msgid.clone(), audience).is_some() {
            return;
        }
        self.order.push_back(msgid);
        if self.order.len() > REMEMBERED
            && let Some(oldest) = self.order.pop_front()
        {
            self.audiences.remove(&oldest);
        }
    }
}

/// Where a line goes: a text or tag message's target, or a multiline batch's.
fn line_target<'a>(message: &LineRef<'a>) -> Option<&'a str> {
    if ["PRIVMSG", "NOTICE", "TAGMSG"]
        .iter()
        .any(|command| message.command_is(command))
    {
        return message.param(0);
    }
    (message.command_is("BATCH")
        && message.param(0).is_some_and(|id| id.starts_with('+'))
        && message.param(1) == Some(crate::cap::MULTILINE.as_str()))
    .then(|| message.param(2))
    .flatten()
}

/// The target and referenced id of a line that threads, reacts to or redacts a message.
fn reference<'a>(message: &LineRef<'a>) -> Option<(&'a str, String)> {
    if message.command_is("REDACT") {
        return Some((message.param(0)?, message.param(1)?.to_owned()));
    }
    let id = message
        .tag("+reply")
        .or_else(|| message.tag("+draft/reply"))?;
    Some((line_target(message)?, id.into_owned()))
}

/// Whether `status` reaches only members ranked at least as high as `original`.
///
/// `PREFIX` lists signs from the highest rank down, and `X#chan` reaches every member ranked
/// `X` or higher. A sign missing from `PREFIX` has no known rank, so only the same sign matches.
fn within(isupport: &Isupport, original: Prefix, status: Prefix) -> bool {
    let prefixes = isupport.prefix();
    let rank = |prefix: Prefix| {
        prefixes
            .iter()
            .position(|known| known.symbol() == prefix.symbol())
    };
    match (rank(status), rank(original)) {
        (Some(new), Some(old)) => new <= old,
        _ => status.symbol() == original.symbol(),
    }
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
                .remember(msgid.into_owned(), Audience { channel, status });
        }
    }

    pub(crate) fn check_line_audience(&self, message: &LineRef<'_>) -> Result<(), RequestError> {
        reference(message).map_or(Ok(()), |(target, id)| self.check_audience(target, &id))
    }

    pub(crate) fn check_text_audience(
        &self,
        target: &str,
        tags: &[(&str, &str)],
    ) -> Result<(), RequestError> {
        tags.iter()
            .filter(|(key, _)| matches!(*key, "+reply" | "+draft/reply"))
            .try_for_each(|(_, id)| self.check_audience(target, id))
    }

    /// Refuses a send to `target` that points at a remembered ops-only line it would widen.
    pub(crate) fn check_audience(&self, target: &str, msgid: &str) -> Result<(), RequestError> {
        let Some(original) = self.ops_only.audiences.get(msgid) else {
            return Ok(());
        };
        let narrower = match self.state.parse_target(target) {
            Ok((Some(status), channel)) => {
                channel == original.channel
                    && within(self.isupport.value(), original.status, status)
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
