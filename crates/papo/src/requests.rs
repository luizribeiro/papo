use std::time::Duration;

use papo_core::{Account, Away, Response};
use papo_proto::{
    AccountName, ChannelName, Command, Isupport, Line, Nick, Numeric, ServerName, WhoxPresence,
    WhoxReply,
};

use crate::Error;

/// Information returned by a `WHOIS` request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Whois {
    /// Nickname reported by the server.
    pub nick: Nick,
    /// Username reported by `RPL_WHOISUSER`; it may start with `~`.
    pub username: Option<String>,
    /// Hostname reported by `RPL_WHOISUSER`.
    pub host: Option<String>,
    /// Real name reported by `RPL_WHOISUSER`.
    pub realname: Option<String>,
    /// Logged-in account name, or `None` when the user is logged out.
    pub account: Option<AccountName>,
    /// Server to which the user is connected.
    pub server: Option<ServerName>,
    /// Visible channels, retaining membership prefixes.
    pub channels: Vec<WhoisChannel>,
    /// The user's presence reported by WHOIS.
    pub away: Away,
    /// Whether the server identifies the user as a bot.
    pub is_bot: bool,
    /// Whether the server identifies the connection as secure.
    pub secure: bool,
    /// Duration for which the user has been idle.
    pub idle: Option<Duration>,
}

/// One channel reported by a `WHOIS` request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct WhoisChannel {
    /// Channel name without membership prefixes.
    pub name: ChannelName,
    /// Membership prefixes in server-advertised order.
    pub prefixes: Vec<char>,
}

/// One user returned by a `WHO` request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct WhoEntry {
    /// Channel associated with this reply, when present.
    pub channel: Option<ChannelName>,
    /// Nickname of the user.
    pub nick: Nick,
    /// Username from `nick!user@host`; it may start with `~`.
    pub username: String,
    /// Visible hostname of the user.
    pub host: String,
    /// The user's account state.
    pub account: Account,
    /// Real name of the user.
    pub realname: String,
    /// The user's presence reported by WHO.
    pub away: Away,
    /// Whether the user is an IRC operator.
    pub is_oper: bool,
    /// Whether the server identifies the user as a bot.
    pub is_bot: bool,
    /// Channel-membership prefixes in wire order.
    pub prefixes: Vec<char>,
}

/// Optional channel names or masks supplied to `LIST`.
///
/// Servers advertise their supported mask filters with the `ELIST` ISUPPORT
/// token. Papo does not require that token: when it is absent, the configured
/// filter is still sent and the server decides how to interpret or reject it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListFilter {
    channels: Vec<String>,
}

impl ListFilter {
    /// Restricts the request to the supplied comma-separated channel masks.
    #[must_use]
    pub fn channels<I, S>(mut self, channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.channels = channels
            .into_iter()
            .map(|channel| channel.as_ref().to_owned())
            .collect();
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
#[non_exhaustive]
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
    let mapping = isupport.casemapping();
    let mut result = Whois {
        nick: mapping.nick(requested_nick),
        username: None,
        host: None,
        realname: None,
        account: None,
        server: None,
        channels: Vec::new(),
        away: Away::Present,
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
                result.nick = mapping.nick(message.param(1).unwrap_or(requested_nick));
                result.username = message.param(2).map(str::to_owned);
                result.host = message.param(3).map(str::to_owned);
                result.realname = message.param(5).map(str::to_owned);
            }
            Numeric::RPL_WHOISSERVER => result.server = message.param(2).map(ServerName::from),
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
            Numeric::RPL_WHOISACCOUNT => {
                result.account = message.param(2).map(|account| mapping.account(account));
            }
            Numeric::RPL_AWAY => {
                result.away = Away::Away {
                    message: message.param(2).map(str::to_owned),
                };
            }
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
        name: isupport.casemapping().channel(&value[offset..]),
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
    let mapping = isupport.casemapping();
    Ok(response
        .lines
        .into_iter()
        .filter_map(|line| {
            let message = line.as_ref();
            if extended {
                let reply = WhoxReply::parse(&message, "%tcuhnfar")?;
                let flags = reply.flags(isupport)?;
                Some(WhoEntry {
                    channel: reply.channel.map(|channel| mapping.channel(channel)),
                    nick: mapping.nick(reply.nick?),
                    username: reply.user?.to_owned(),
                    host: reply.host?.to_owned(),
                    account: reply.account.map_or(Account::LoggedOut, |account| {
                        Account::LoggedIn(mapping.account(account))
                    }),
                    realname: reply.realname?.to_owned(),
                    away: match flags.presence {
                        Some(WhoxPresence::Here) => Away::Present,
                        Some(WhoxPresence::Gone) => Away::Away { message: None },
                        None => Away::Unknown,
                    },
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

fn plain_who(message: &papo_proto::LineRef<'_>, isupport: &Isupport) -> Option<WhoEntry> {
    (message.command() == Command::Numeric(Numeric::RPL_WHOREPLY)).then_some(())?;
    let flags = message.param(6)?;
    let prefixes = isupport.prefix();
    Some(WhoEntry {
        channel: message
            .param(1)
            .filter(|value| *value != "*")
            .map(|channel| isupport.casemapping().channel(channel)),
        username: message.param(2)?.to_owned(),
        host: message.param(3)?.to_owned(),
        nick: isupport.casemapping().nick(message.param(5)?),
        account: Account::Unknown,
        realname: message.param(7)?.split_once(' ')?.1.to_owned(),
        away: if flags.contains('G') {
            Away::Away { message: None }
        } else if flags.contains('H') {
            Away::Present
        } else {
            Away::Unknown
        },
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

pub(crate) fn list(response: Response, isupport: &Isupport) -> Result<Vec<ListEntry>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    Ok(list_entries(response.lines, isupport))
}

fn list_entries(lines: impl IntoIterator<Item = Line>, isupport: &Isupport) -> Vec<ListEntry> {
    lines
        .into_iter()
        .filter_map(|line| {
            let message = line.as_ref();
            if message.command() != Command::Numeric(Numeric::RPL_LIST) {
                return None;
            }
            Some(ListEntry {
                name: isupport.casemapping().channel(message.param(1)?),
                members_count: message.param(2)?.parse().ok()?,
                topic: message.param(3)?.to_owned(),
            })
        })
        .take(10_000)
        .collect()
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::Instant;

    use papo_core::{Account, Away, Config, Event, ResponseMatcher, Session};
    use papo_proto::{Casemapping, Isupport, Line};

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
        let hidden = Line::new(
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
        let visible = Line::new(
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
        assert_eq!(
            (hidden.account, hidden.away),
            (Account::Unknown, Away::Present)
        );
        assert_eq!(
            (visible.account, visible.away),
            (Account::Unknown, Away::Away { message: None })
        );
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
                Line::new("MODE", ["#papo", "+t"]),
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
            Line::new(
                "322",
                ["papo", &format!("#papo-{index}"), "42", "Ergonomic IRC"],
            )
        });
        let entries = list_entries(lines, &Isupport::default());
        assert_eq!(entries.len(), 10_000);
        assert_eq!(entries.last().unwrap().name, "#papo-9999");
    }

    #[test]
    fn reply_names_capture_the_advertised_casemapping() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CASEMAPPING=ascii"]);
        let channel = whois_channel("@#Papo[dev]", &isupport);
        let who = Line::new(
            "352",
            [
                "papo",
                "#Papo[dev]",
                "~alice",
                "staff.example",
                "ergo.test",
                "Alice[bot]",
                "H@",
                "0 Alice Example",
            ],
        );
        let who = plain_who(&who.as_ref(), &isupport).unwrap();

        assert_eq!(channel.name.casemapping(), Casemapping::Ascii);
        assert_eq!(who.channel.unwrap().casemapping(), Casemapping::Ascii);
        assert_eq!(who.nick.casemapping(), Casemapping::Ascii);
    }
}
