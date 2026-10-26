use std::time::Duration;

use papo_core::{Account, Away, Response};
use papo_proto::{
    ChannelName, Command, Isupport, Line, Nick, Numeric, ServerName, WhoxPresence, WhoxReply,
};

use crate::{Error, Oper, Prefix, Rank};

pub(crate) const WHOX_FIELDS: &str = "%tcuhnfar,152";

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
    /// The user's account state.
    pub account: Account,
    /// Server to which the user is connected.
    pub server: Option<ServerName>,
    /// Visible channels, retaining membership prefixes.
    pub channels: Vec<WhoisChannel>,
    /// The user's presence reported by WHOIS.
    pub away: Away,
    /// Whether the server identifies the user as a bot.
    pub is_bot: Option<bool>,
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
    pub prefixes: Vec<Prefix>,
}

impl WhoisChannel {
    /// Returns the highest standard rank held in this channel.
    #[must_use]
    pub fn rank(&self) -> Option<Rank> {
        highest_rank(&self.prefixes)
    }
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
    /// The user's IRC operator identity, when reported.
    pub oper: Option<Oper>,
    /// Whether the server identifies the user as a bot.
    pub is_bot: Option<bool>,
    /// Channel-membership prefixes in wire order.
    pub prefixes: Vec<Prefix>,
}

impl WhoEntry {
    /// Returns the highest standard channel rank reported for this user.
    #[must_use]
    pub fn rank(&self) -> Option<Rank> {
        highest_rank(&self.prefixes)
    }
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
    origin: crate::Origin,
) -> Result<Whois, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    let mapping = isupport.casemapping();
    let mut result = Whois {
        nick: origin.__nick(mapping, requested_nick),
        username: None,
        host: None,
        realname: None,
        account: Account::LoggedOut,
        server: None,
        channels: Vec::new(),
        away: Away::Present,
        is_bot: isupport.bot_mode().map(|_| false),
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
                result.nick = origin.__nick(mapping, message.param(1).unwrap_or(requested_nick));
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
                    .map(|value| whois_channel(value, isupport, origin)),
            ),
            Numeric::RPL_WHOISACCOUNT => {
                if let Some(account) = message.param(2) {
                    result.account = Account::LoggedIn(origin.__account(mapping, account));
                }
            }
            Numeric::RPL_AWAY => {
                result.away = Away::__away_value(message.param(2).map(str::to_owned));
            }
            Numeric::RPL_WHOISBOT => result.is_bot = Some(true),
            Numeric::RPL_WHOISSECURE => result.secure = true,
            _ => {}
        }
    }
    Ok(result)
}

fn whois_channel(value: &str, isupport: &Isupport, origin: crate::Origin) -> WhoisChannel {
    let symbols: String = value
        .chars()
        .take_while(|symbol| {
            isupport
                .prefix()
                .iter()
                .any(|prefix| prefix.symbol() == *symbol)
        })
        .collect();
    let offset = symbols.len();
    WhoisChannel {
        name: origin.__channel(isupport.casemapping(), &value[offset..]),
        prefixes: ordered_prefixes(&symbols, isupport),
    }
}

pub(crate) fn who(
    response: Response,
    isupport: &Isupport,
    origin: crate::Origin,
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
                let reply = WhoxReply::parse(&message, WHOX_FIELDS)?;
                let flags = reply.flags(isupport)?;
                Some(WhoEntry {
                    channel: reply
                        .channel
                        .map(|channel| origin.__channel(mapping, channel)),
                    nick: origin.__nick(mapping, reply.nick?),
                    username: reply.user?.to_owned(),
                    host: reply.host?.to_owned(),
                    account: reply.account.map_or(Account::LoggedOut, |account| {
                        Account::LoggedIn(origin.__account(mapping, account))
                    }),
                    realname: reply.realname?.to_owned(),
                    away: match flags.presence {
                        Some(WhoxPresence::Here) => Away::Present,
                        Some(WhoxPresence::Gone) => Away::__away_value(None),
                        None => Away::Unknown,
                    },
                    oper: flags.oper.then_some(Oper { name: None }),
                    is_bot: if flags.bot {
                        Some(true)
                    } else {
                        isupport.bot_mode().map(|_| false)
                    },
                    prefixes: ordered_prefixes(
                        &flags.prefixes.iter().collect::<String>(),
                        isupport,
                    ),
                })
            } else {
                plain_who(&message, isupport, origin)
            }
        })
        .collect())
}

fn plain_who(
    message: &papo_proto::LineRef<'_>,
    isupport: &Isupport,
    origin: crate::Origin,
) -> Option<WhoEntry> {
    (message.command() == Command::Numeric(Numeric::RPL_WHOREPLY)).then_some(())?;
    let flags = message.param(6)?;
    let prefixes = isupport.prefix();
    Some(WhoEntry {
        channel: message
            .param(1)
            .filter(|value| *value != "*")
            .map(|channel| origin.__channel(isupport.casemapping(), channel)),
        username: message.param(2)?.to_owned(),
        host: message.param(3)?.to_owned(),
        nick: origin.__nick(isupport.casemapping(), message.param(5)?),
        account: Account::Unknown,
        realname: message.param(7)?.split_once(' ')?.1.to_owned(),
        away: if flags.contains('G') {
            Away::__away_value(None)
        } else if flags.contains('H') {
            Away::Present
        } else {
            Away::Unknown
        },
        oper: flags.contains('*').then_some(Oper { name: None }),
        is_bot: if flags.contains('B') {
            Some(true)
        } else {
            isupport.bot_mode().map(|_| false)
        },
        prefixes: prefixes
            .into_iter()
            .filter(|prefix| flags.contains(prefix.symbol()))
            .collect(),
    })
}

fn ordered_prefixes(symbols: &str, isupport: &Isupport) -> Vec<Prefix> {
    isupport
        .prefix()
        .into_iter()
        .filter(|prefix| symbols.contains(prefix.symbol()))
        .collect()
}

fn highest_rank(prefixes: &[Prefix]) -> Option<Rank> {
    prefixes.iter().filter_map(|prefix| prefix.rank()).max()
}

pub(crate) fn empty(response: &Response) -> Result<(), Error> {
    response
        .error()
        .map_or(Ok(()), |error| Err(Error::server(error)))
}

pub(crate) fn list(
    response: Response,
    isupport: &Isupport,
    origin: crate::Origin,
) -> Result<Vec<ListEntry>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::server(error));
    }
    Ok(list_entries(response.lines, isupport, origin))
}

fn list_entries(
    lines: impl IntoIterator<Item = Line>,
    isupport: &Isupport,
    origin: crate::Origin,
) -> Vec<ListEntry> {
    lines
        .into_iter()
        .filter_map(|line| {
            let message = line.as_ref();
            if message.command() != Command::Numeric(Numeric::RPL_LIST) {
                return None;
            }
            Some(ListEntry {
                name: origin.__channel(isupport.casemapping(), message.param(1)?),
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
        let mut support = Isupport::default();
        support.apply_tokens(["PREFIX=(qaohv)~&@%+"]);
        let channel = whois_channel("~@#papo", &support, crate::Origin::NONE);

        assert_eq!(channel.name, "#papo");
        assert_eq!(
            channel
                .prefixes
                .iter()
                .map(|prefix| prefix.symbol())
                .collect::<Vec<_>>(),
            ['~', '@']
        );
        assert_eq!(channel.rank(), Some(crate::Rank::Owner));
    }

    #[test]
    fn plain_who_filters_placeholder_channels_and_non_prefix_flags() {
        let isupport = Isupport::default();
        let hidden = Line::new("352").with_params([
            "papo",
            "*",
            "~alice",
            "staff.example",
            "ergo.test",
            "alice",
            "H*@",
            "0 Alice Example",
        ]);
        let visible = Line::new("352").with_params([
            "papo",
            "#rust",
            "~bob",
            "users.example",
            "ergo.test",
            "bob",
            "G+?",
            "0 Bob Example",
        ]);

        let hidden = plain_who(&hidden.as_ref(), &isupport, crate::Origin::NONE).unwrap();
        let visible = plain_who(&visible.as_ref(), &isupport, crate::Origin::NONE).unwrap();
        assert_eq!(
            (&hidden.account, &hidden.away),
            (&Account::Unknown, &Away::Present)
        );
        assert_eq!(
            (&visible.account, &visible.away),
            (&Account::Unknown, &Away::__away_value(None))
        );
        assert_eq!(hidden.channel, None);
        assert_eq!(hidden.prefixes[0].symbol(), '@');
        assert!(hidden.oper.is_some());
        assert_eq!(hidden.rank(), Some(crate::Rank::Op));
        assert_eq!(
            (visible.channel.as_deref(), visible.prefixes[0].symbol()),
            (Some("#rust"), '+')
        );
        assert_eq!(visible.rank(), Some(crate::Rank::Voice));
    }

    #[test]
    fn empty_propagates_a_numeric_server_error() {
        let now = Instant::now();
        let mut session = Session::new(Config::default());
        session
            .request_with_matcher(
                now,
                Line::new("MODE").with_params(["#papo", "+t"]),
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
            Line::new("322").with_params(["papo", &format!("#papo-{index}"), "42", "Ergonomic IRC"])
        });
        let entries = list_entries(lines, &Isupport::default(), crate::Origin::NONE);
        assert_eq!(entries.len(), 10_000);
        assert_eq!(entries.last().unwrap().name, "#papo-9999");
    }

    #[test]
    fn reply_names_capture_the_advertised_casemapping() {
        let mut isupport = Isupport::default();
        isupport.apply_tokens(["CASEMAPPING=ascii"]);
        let origin = crate::Origin::__new(73);
        let channel = whois_channel("@#Papo[dev]", &isupport, origin);
        let who = Line::new("352").with_params([
            "papo",
            "#Papo[dev]",
            "~alice",
            "staff.example",
            "ergo.test",
            "Alice[bot]",
            "H@",
            "0 Alice Example",
        ]);
        let who = plain_who(&who.as_ref(), &isupport, origin).unwrap();

        assert_eq!(channel.name.casemapping(), Casemapping::Ascii);
        assert_eq!(who.channel.unwrap().casemapping(), Casemapping::Ascii);
        assert_eq!(who.nick.casemapping(), Casemapping::Ascii);
        assert_eq!(channel.name.origin(), origin);
        assert_eq!(who.nick.origin(), origin);
    }
}
