use std::time::Duration;

use papo_core::Response;
use papo_proto::{
    AccountName, ChannelName, Command, Isupport, Mask, Message, Nick, Numeric, WhoxPresence,
    WhoxReply,
};

use crate::Error;

/// Information returned by a `WHOIS` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Whois {
    /// Nickname reported by the server.
    pub nick: Nick,
    /// Username reported by `RPL_WHOISUSER`.
    pub user: Option<String>,
    /// Hostname reported by `RPL_WHOISUSER`.
    pub host: Option<String>,
    /// Real name reported by `RPL_WHOISUSER`.
    pub realname: Option<String>,
    /// Logged-in account name.
    pub account: Option<AccountName>,
    /// Server to which the user is connected.
    pub server: Option<String>,
    /// Visible channels, retaining membership prefixes.
    pub channels: Vec<WhoisChannel>,
    /// Away message, when the user is away.
    pub away: Option<String>,
    /// Whether the server identifies the user as a bot.
    pub is_bot: bool,
    /// Whether the server identifies the connection as secure.
    pub secure: bool,
    /// Duration for which the user has been idle.
    pub idle: Option<Duration>,
}

/// One channel reported by a `WHOIS` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhoisChannel {
    /// Channel name without membership prefixes.
    pub name: ChannelName,
    /// Membership prefixes in server-advertised order.
    pub prefixes: Vec<char>,
}

/// One user returned by a `WHO` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhoEntry {
    /// Channel associated with this reply, when present.
    pub channel: Option<ChannelName>,
    /// Nickname of the user.
    pub nick: Nick,
    /// Username of the user.
    pub user: String,
    /// Visible hostname of the user.
    pub host: String,
    /// Logged-in account name.
    pub account: Option<AccountName>,
    /// Real name of the user.
    pub realname: String,
    /// Whether the user is away.
    pub away: bool,
    /// Whether the user is an IRC operator.
    pub is_oper: bool,
    /// Whether the server identifies the user as a bot.
    pub is_bot: bool,
    /// Channel-membership prefixes in wire order.
    pub prefixes: Vec<char>,
}

/// Optional channel names or masks supplied to `LIST`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListFilter {
    channels: Vec<Mask>,
}

impl ListFilter {
    /// Restricts the request to the supplied comma-separated channel masks.
    #[must_use]
    pub fn channels<I, S>(mut self, channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Mask>,
    {
        self.channels = channels.into_iter().map(Into::into).collect();
        self
    }

    pub(crate) fn parameter(&self) -> Option<String> {
        (!self.channels.is_empty()).then(|| {
            self.channels
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(",")
        })
    }
}

/// One channel returned by `LIST`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListEntry {
    /// Channel name.
    pub name: ChannelName,
    /// Number of visible members reported by the server.
    pub members_count: usize,
    /// Channel topic.
    pub topic: String,
}

pub(crate) fn whois(
    response: Response,
    requested_nick: &str,
    isupport: &Isupport,
) -> Result<Whois, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    let mut result = Whois {
        nick: requested_nick.into(),
        user: None,
        host: None,
        realname: None,
        account: None,
        server: None,
        channels: Vec::new(),
        away: None,
        is_bot: false,
        secure: false,
        idle: None,
    };
    for line in response.lines {
        let message = line.as_ref();
        let Command::Numeric(numeric) = message.command() else {
            continue;
        };
        match numeric {
            Numeric::RPL_WHOISUSER => {
                result.nick = message.param(1).unwrap_or(requested_nick).into();
                result.user = message.param(2).map(str::to_owned);
                result.host = message.param(3).map(str::to_owned);
                result.realname = message.param(5).map(str::to_owned);
            }
            Numeric::RPL_WHOISSERVER => result.server = message.param(2).map(str::to_owned),
            Numeric::RPL_WHOISIDLE => {
                result.idle = message
                    .param(2)
                    .and_then(|seconds| seconds.parse().ok())
                    .map(Duration::from_secs);
            }
            Numeric::RPL_WHOISCHANNELS => result.channels.extend(
                message
                    .param(2)
                    .into_iter()
                    .flat_map(str::split_ascii_whitespace)
                    .map(|value| whois_channel(value, isupport)),
            ),
            Numeric::RPL_WHOISACCOUNT => result.account = message.param(2).map(AccountName::from),
            Numeric::RPL_AWAY => result.away = message.param(2).map(str::to_owned),
            Numeric::RPL_WHOISBOT => result.is_bot = true,
            Numeric::RPL_WHOISSECURE => result.secure = true,
            _ => {}
        }
    }
    Ok(result)
}

fn whois_channel(value: &str, isupport: &Isupport) -> WhoisChannel {
    let prefixes: Vec<_> = value
        .chars()
        .take_while(|symbol| {
            isupport
                .prefix()
                .iter()
                .any(|prefix| prefix.symbol == *symbol)
        })
        .collect();
    let offset = prefixes.iter().map(|symbol| symbol.len_utf8()).sum();
    WhoisChannel {
        name: value[offset..].into(),
        prefixes,
    }
}

pub(crate) fn who(
    response: Response,
    isupport: &Isupport,
    extended: bool,
) -> Result<Vec<WhoEntry>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    Ok(response
        .lines
        .into_iter()
        .filter_map(|line| {
            let message = line.as_ref();
            if extended {
                let reply = WhoxReply::parse(&message, "%tcuhnfar")?;
                let flags = reply.flags(isupport)?;
                Some(WhoEntry {
                    channel: reply.channel.map(ChannelName::from),
                    nick: reply.nick?.into(),
                    user: reply.user?.to_owned(),
                    host: reply.host?.to_owned(),
                    account: reply.account.map(AccountName::from),
                    realname: reply.realname?.to_owned(),
                    away: flags.presence == Some(WhoxPresence::Gone),
                    is_oper: flags.oper,
                    is_bot: flags.bot,
                    prefixes: flags.prefixes,
                })
            } else {
                plain_who(&message, isupport)
            }
        })
        .collect())
}

fn plain_who(message: &papo_proto::MessageRef<'_>, isupport: &Isupport) -> Option<WhoEntry> {
    (message.command() == Command::Numeric(Numeric::RPL_WHOREPLY)).then_some(())?;
    let flags = message.param(6)?;
    let prefixes = isupport.prefix();
    Some(WhoEntry {
        channel: message
            .param(1)
            .filter(|value| *value != "*")
            .map(ChannelName::from),
        user: message.param(2)?.to_owned(),
        host: message.param(3)?.to_owned(),
        nick: message.param(5)?.into(),
        account: None,
        realname: message.param(7)?.split_once(' ')?.1.to_owned(),
        away: flags.contains('G'),
        is_oper: flags.contains('*'),
        is_bot: flags.contains('B'),
        prefixes: flags
            .chars()
            .filter(|flag| prefixes.iter().any(|prefix| prefix.symbol == *flag))
            .collect(),
    })
}

pub(crate) fn empty(response: &Response) -> Result<(), Error> {
    response
        .error()
        .map_or(Ok(()), |error| Err(Error::server(error)))
}

pub(crate) fn topic(response: &Response) -> Result<Option<String>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    Ok(response
        .lines
        .iter()
        .find_map(|line| {
            let message = line.as_ref();
            if message.command() == Command::Numeric(Numeric::RPL_NOTOPIC) {
                Some(None)
            } else if message.command() == Command::Numeric(Numeric::RPL_TOPIC) {
                Some(message.param(2).map(str::to_owned))
            } else if message.command_is("TOPIC") {
                Some(message.param(1).map(str::to_owned))
            } else {
                None
            }
        })
        .flatten())
}

pub(crate) fn list(response: Response) -> Result<Vec<ListEntry>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    Ok(list_entries(response.lines))
}

fn list_entries(lines: impl IntoIterator<Item = Message>) -> Vec<ListEntry> {
    lines
        .into_iter()
        .filter_map(|line| {
            let message = line.as_ref();
            if message.command() != Command::Numeric(Numeric::RPL_LIST) {
                return None;
            }
            Some(ListEntry {
                name: message.param(1)?.into(),
                members_count: message.param(2)?.parse().ok()?,
                topic: message.param(3)?.to_owned(),
            })
        })
        .take(10_000)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use papo_core::{Config, Event, ResponseMatcher, Session};
    use papo_proto::{Isupport, Message};

    use super::{ListFilter, empty, list_entries, plain_who, whois_channel};
    use crate::Error;

    #[test]
    fn list_filter_encodes_empty_and_comma_separated_parameters() {
        assert_eq!(ListFilter::default().parameter(), None);
        assert_eq!(
            ListFilter::default()
                .channels(["#papo", "#rust-*", "##chat"])
                .parameter()
                .as_deref(),
            Some("#papo,#rust-*,##chat")
        );
    }

    #[test]
    fn whois_channels_separate_advertised_membership_prefixes() {
        let channel = whois_channel("@+#papo", &Isupport::default());

        assert_eq!(channel.name, "#papo");
        assert_eq!(channel.prefixes, vec!['@', '+']);
    }

    #[test]
    fn plain_who_filters_placeholder_channels_and_non_prefix_flags() {
        let isupport = Isupport::default();
        let hidden = Message::new(
            "352",
            [
                "papo",
                "*",
                "~alice",
                "staff.example",
                "ergo.test",
                "alice",
                "H@!",
                "0 Alice Example",
            ],
        );
        let visible = Message::new(
            "352",
            [
                "papo",
                "#rust",
                "~bob",
                "users.example",
                "ergo.test",
                "bob",
                "G+?",
                "0 Bob Example",
            ],
        );

        let hidden = plain_who(&hidden.as_ref(), &isupport).unwrap();
        let visible = plain_who(&visible.as_ref(), &isupport).unwrap();
        assert_eq!((hidden.channel, hidden.prefixes), (None, vec!['@']));
        assert_eq!(
            (visible.channel.as_deref(), visible.prefixes),
            (Some("#rust"), vec!['+'])
        );
    }

    #[test]
    fn empty_propagates_a_numeric_server_error() {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session
            .request_with_matcher(
                now,
                Message::new("MODE", ["#papo", "+t"]),
                ResponseMatcher::Mode("#papo".into()),
            )
            .unwrap();
        session.handle_line(
            now,
            b":ergo.test 482 papo #papo :You're not a channel operator",
        );
        session.handle_line(now, b":ergo.test PONG ergo.test :papo-mode-0");
        let response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) => Some(response),
                _ => None,
            })
            .unwrap();

        assert!(matches!(
            empty(&response),
            Err(Error::Server(error)) if error.code() == "482"
        ));
    }

    #[test]
    fn list_retains_at_most_the_literal_entry_limit() {
        let lines = (0..10_001).map(|index| {
            Message::new(
                "322",
                ["papo", &format!("#papo-{index}"), "42", "Ergonomic IRC"],
            )
        });
        let entries = list_entries(lines);
        assert_eq!(entries.len(), 10_000);
        assert_eq!(entries.last().unwrap().name, "#papo-9999");
    }
}
