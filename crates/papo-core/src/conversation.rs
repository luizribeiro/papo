use papo_proto::{InvalidIdentifier, Isupport, LineRef, Source};

use crate::{Conversation, names_equal};

pub(crate) fn classify_line(
    line: &LineRef<'_>,
    own_nick: &str,
    isupport: &Isupport,
) -> Result<(Option<Vec<char>>, Conversation), InvalidIdentifier> {
    classify_target(
        line.param(0).unwrap_or(""),
        line.source(),
        own_nick,
        isupport,
    )
}

pub(crate) fn classify_target(
    target: &str,
    source: Option<Source<'_>>,
    own_nick: &str,
    isupport: &Isupport,
) -> Result<(Option<Vec<char>>, Conversation), InvalidIdentifier> {
    let status_len = target
        .chars()
        .take_while(|prefix| {
            isupport
                .statusmsg()
                .is_some_and(|status| status.contains(*prefix))
        })
        .map(char::len_utf8)
        .sum();
    let (status, target) = target.split_at(status_len);
    let status = (!status.is_empty()).then(|| status.chars().collect());
    let mapping = isupport.casemapping();

    let conversation = if isupport.is_channel(target) {
        let channel = mapping.channel(target);
        channel.check(isupport)?;
        Conversation::Channel(channel)
    } else if target.starts_with('$') {
        let mask = mapping.mask(target);
        mask.check()?;
        Conversation::Broadcast(mask)
    } else if target == "*" || matches!(source, Some(Source::Server(_))) {
        Conversation::Server
    } else {
        let other = match source {
            Some(Source::User { nick, .. }) if names_equal(mapping, target, own_nick) => nick,
            _ => target,
        };
        let nick = mapping.nick(other);
        nick.check(isupport)?;
        Conversation::Private(nick)
    };
    Ok((status, conversation))
}
