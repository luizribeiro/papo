use papo_proto::{Casemapping, InvalidIdentifier, Isupport, LineRef, Source};

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
        isupport.casemapping(),
        isupport,
    )
}

pub(crate) fn classify_target(
    target: &str,
    source: Option<Source<'_>>,
    own_nick: &str,
    mapping: Casemapping,
    isupport: &Isupport,
) -> Result<(Option<Vec<char>>, Conversation), InvalidIdentifier> {
    let mut remainder = target;
    let mut status = Vec::new();
    loop {
        if isupport.is_channel(remainder) {
            let channel = mapping.channel(remainder);
            channel.check(isupport)?;
            return Ok((
                (!status.is_empty()).then_some(status),
                Conversation::Channel(channel),
            ));
        }
        let Some(prefix) = remainder.chars().next() else {
            break;
        };
        if !isupport
            .statusmsg()
            .is_some_and(|advertised| advertised.contains(prefix))
        {
            break;
        }
        status.push(prefix);
        remainder = &remainder[prefix.len_utf8()..];
    }
    if let Some(prefix) = status.first().copied() {
        return Err(InvalidIdentifier::InvalidPrefix(prefix));
    }

    let conversation = if target.starts_with('$') {
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
    Ok((None, conversation))
}
