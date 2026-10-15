use crate::{Command, MessageRef, Numeric};

const FIELDS: &str = "tcuihsnfdlaor";

/// A typed view of an `RPL_WHOSPCRPL` (`354`) reply.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WhoxReply<'a> {
    /// The query token (`t`).
    pub token: Option<&'a str>,
    /// The channel (`c`), or `None` for the `*` placeholder.
    pub channel: Option<&'a str>,
    /// The username (`u`).
    pub user: Option<&'a str>,
    /// The visible IP address (`i`), or `None` when hidden.
    pub ip: Option<&'a str>,
    /// The hostname (`h`).
    pub host: Option<&'a str>,
    /// The server name (`s`).
    pub server: Option<&'a str>,
    /// The nickname (`n`).
    pub nick: Option<&'a str>,
    /// The unparsed flags (`f`).
    pub raw_flags: Option<&'a str>,
    /// The hop count (`d`).
    pub hopcount: Option<&'a str>,
    /// The idle time (`l`).
    pub idle: Option<&'a str>,
    /// The account name (`a`), or `None` when logged out.
    pub account: Option<&'a str>,
    /// The channel operator level (`o`).
    pub oplevel: Option<&'a str>,
    /// The real name (`r`).
    pub realname: Option<&'a str>,
}

impl<'a> WhoxReply<'a> {
    /// Maps a `354` reply onto the requested WHOX field selector.
    ///
    /// The selector may include its leading `%` and `,<token>` suffix. Unknown
    /// or repeated field letters and mismatched reply arity are rejected.
    #[must_use]
    pub fn parse(message: &MessageRef<'a>, fields: &str) -> Option<Self> {
        if message.command() != Command::Numeric(Numeric::RPL_WHOSPCRPL) {
            return None;
        }
        let requested = fields.strip_prefix('%').unwrap_or(fields);
        let requested = requested
            .split_once(',')
            .map_or(requested, |(fields, _)| fields);
        if selector_is_invalid(requested) {
            return None;
        }

        let mut values = message.params();
        values.next()?;
        let mut reply = Self::default();
        for field in FIELDS.chars().filter(|field| requested.contains(*field)) {
            let value = values.next()?;
            match field {
                't' => reply.token = Some(value),
                'c' => reply.channel = (value != "*").then_some(value),
                'u' => reply.user = Some(value),
                'i' => reply.ip = (value != "255.255.255.255").then_some(value),
                'h' => reply.host = Some(value),
                's' => reply.server = Some(value),
                'n' => reply.nick = Some(value),
                'f' => reply.raw_flags = Some(value),
                'd' => reply.hopcount = Some(value),
                'l' => reply.idle = Some(value),
                'a' => reply.account = (value != "0").then_some(value),
                'o' => reply.oplevel = Some(value),
                'r' => reply.realname = Some(value),
                _ => return None,
            }
        }
        values.next().is_none().then_some(reply)
    }
}

fn selector_is_invalid(selector: &str) -> bool {
    selector.is_empty()
        || selector.chars().any(|field| !FIELDS.contains(field))
        || selector
            .char_indices()
            .any(|(index, field)| selector[index + field.len_utf8()..].contains(field))
}

#[cfg(test)]
mod tests {
    use super::selector_is_invalid;

    #[test]
    fn validates_whox_field_selectors() {
        assert!(selector_is_invalid(""));
        assert!(selector_is_invalid("x"));
        assert!(selector_is_invalid("nn"));
        assert!(!selector_is_invalid("tcuhnfar"));
    }
}
