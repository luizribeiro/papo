use papo_proto::{Command, InvalidIdentifier, Isupport, LineRef, Source};

use crate::{Conversation, Names, names_equal};

pub(crate) enum LineTarget {
    Conversation(Conversation),
    Server,
    Broadcast,
}

impl LineTarget {
    pub(crate) fn into_conversation(self) -> Option<Conversation> {
        match self {
            Self::Conversation(conversation) => Some(conversation),
            Self::Server | Self::Broadcast => None,
        }
    }
}

pub(crate) fn classify_line(
    line: &LineRef<'_>,
    own_nick: &str,
    isupport: &Isupport,
) -> Result<(Option<Vec<char>>, LineTarget), InvalidIdentifier> {
    classify_line_with_names(
        line,
        own_nick,
        isupport,
        Names::new(isupport.casemapping(), papo_proto::Origin::NONE),
    )
}

pub(crate) fn classify_line_with_names(
    line: &LineRef<'_>,
    own_nick: &str,
    isupport: &Isupport,
    names: Names,
) -> Result<(Option<Vec<char>>, LineTarget), InvalidIdentifier> {
    if line.command_is("WALLOPS") || matches!(line.command(), Command::Numeric(_)) {
        return Ok((None, LineTarget::Server));
    }
    classify_target_with_names(
        line.param(0).unwrap_or(""),
        line.source(),
        own_nick,
        names,
        isupport,
    )
}

pub(crate) fn classify_target_with_names(
    target: &str,
    source: Option<Source<'_>>,
    own_nick: &str,
    mapping: Names,
    isupport: &Isupport,
) -> Result<(Option<Vec<char>>, LineTarget), InvalidIdentifier> {
    let mut remainder = target;
    let mut status = Vec::new();
    loop {
        if isupport.is_channel(remainder) {
            let channel = mapping.channel(remainder);
            channel.check(isupport)?;
            return Ok((
                (!status.is_empty()).then_some(status),
                LineTarget::Conversation(Conversation::Channel(channel)),
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

    let target = if target.starts_with('$') {
        let mask = mapping.mask(target);
        mask.check()?;
        LineTarget::Broadcast
    } else if target == "*" || matches!(source, Some(Source::Server(_))) {
        LineTarget::Server
    } else {
        let other = match source {
            Some(Source::User { nick, .. }) if names_equal(mapping, target, own_nick) => nick,
            _ => target,
        };
        let nick = mapping.nick(other);
        nick.check(isupport)?;
        LineTarget::Conversation(Conversation::Private(nick))
    };
    Ok((None, target))
}
