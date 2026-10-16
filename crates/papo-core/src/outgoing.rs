use crate::{Capabilities, State};

const UNKNOWN_HOST_LEN: usize = 63;

/// Kind of an outgoing text message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextKind {
    /// A `PRIVMSG` message.
    Privmsg,
    /// A `NOTICE` message.
    Notice,
}

impl TextKind {
    pub(super) const fn command(self) -> &'static str {
        match self {
            Self::Privmsg => "PRIVMSG",
            Self::Notice => "NOTICE",
        }
    }
}

pub(super) fn text_budget(state: &State, kind: TextKind, target: &str) -> usize {
    // Reserve the longest plausible IRC hostname until the server reveals the visible host.
    let host_len = state.host().map_or(UNKNOWN_HOST_LEN, str::len);
    let source_len = 1 + state.nick().len() + 1 + state.username().len() + 1 + host_len + 1;
    let message_len = kind.command().len() + 1 + target.len() + 2;
    512usize.saturating_sub(source_len + message_len + 2)
}

pub(super) fn first_line_only(key: &str) -> bool {
    matches!(
        key,
        "+reply" | "+draft/reply" | "+channel-context" | "+draft/channel-context"
    )
}

#[derive(Clone, Copy)]
pub(super) struct MultilineLimits {
    pub(super) max_bytes: usize,
    pub(super) max_lines: usize,
}

pub(super) fn multiline_limits(caps: &Capabilities) -> Option<MultilineLimits> {
    let value = caps.get("draft/multiline")?.as_deref()?;
    let number = |name| {
        value.split(',').find_map(|item| {
            let (key, value) = item.split_once('=')?;
            (key == name).then(|| value.parse().ok()).flatten()
        })
    };
    let limits = MultilineLimits {
        max_bytes: number("max-bytes")?,
        max_lines: number("max-lines").unwrap_or(usize::MAX),
    };
    (limits.max_bytes > 0 && limits.max_lines > 0).then_some(limits)
}

#[cfg(test)]
mod tests {
    use web_time::Instant;

    use super::TextKind;
    use crate::{Config, Session};

    #[test]
    fn budgets_from_the_current_visible_prefix() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("emily").username("em"));
        assert_eq!(session.text_budget(TextKind::Privmsg, "#papo"), 421);
        assert_eq!(session.text_budget(TextKind::Notice, "#papo"), 422);

        session.handle_line(now, b":emily!em@localhost CHGHOST ~em users/emily");
        assert_eq!(session.text_budget(TextKind::Privmsg, "#papo"), 472);

        session.handle_line(
            now,
            b":ergo.test 396 emily gateway/tor-sasl :is now your displayed host",
        );
        assert_eq!(session.text_budget(TextKind::Privmsg, "#papo"), 467);
    }
}
