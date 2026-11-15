use std::{future::IntoFuture, time::Duration};

use papo_core::{Account, Away, Response, ResponseMatcher};
use papo_proto::{
    ChannelName, Command, Isupport, Line, LineRef, Nick, Numeric, ServerName, WhoxPresence,
    WhoxReply,
};

use crate::{Error, Prefix, Rank};

pub(crate) const WHOX_FIELDS: &str = "%tcuhnfar,152";

/// The nickname change the server confirmed by echoing `NICK`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NickOutcome {
    /// The new nickname, spelled as the server echoed it.
    pub nick: Nick,
    /// The echo's `msgid` and server time, which Ergo assigns to a nickname change.
    pub confirmation: crate::Confirmation,
}

/// The realname change the server confirmed by echoing `SETNAME`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RealnameOutcome {
    /// The realname the server accepted, which may differ from the one sent.
    pub realname: String,
    /// The echo's `msgid` and server time, when the server tagged it with a `msgid`.
    pub confirmation: crate::Confirmation,
}

/// The topic change the server confirmed by echoing `TOPIC`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TopicOutcome {
    /// The topic the server accepted, which may be cut to `TOPICLEN`; empty when cleared.
    pub topic: String,
    /// The echo's `msgid` and server time, which Ergo assigns to a topic change.
    pub confirmation: crate::Confirmation,
}

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
    ///
    /// [`Account::LoggedIn`] means the server sent `RPL_WHOISACCOUNT` (`330`). A completed
    /// WHOIS reply without that numeric leaves this [`Account::Unknown`], because WHOIS has no
    /// portable reply that proves the user is logged out.
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
    pub channel: Option<crate::ChannelHandle>,
    /// User reported by the reply.
    pub user: crate::UserHandle,
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
    /// Whether the reply identifies the user as an IRC operator.
    pub is_oper: bool,
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

/// Builds a request for visible channels.
///
/// Servers advertise supported mask filters with the `ELIST` ISUPPORT token. Papo sends
/// configured channel masks even when `ELIST` is absent, leaving interpretation to the server.
#[must_use = "requests do nothing unless awaited"]
pub struct ListRequest {
    client: crate::Client,
    channels: Vec<String>,
}

impl ListRequest {
    pub(crate) fn new(client: crate::Client) -> Self {
        Self {
            client,
            channels: Vec::new(),
        }
    }

    /// Restricts the request to the supplied comma-separated channel masks.
    pub fn channels<I, S>(mut self, channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.channels = channels.into_iter().map(Into::into).collect();
        self
    }
}

impl std::fmt::Debug for ListRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("ListRequest", formatter)
    }
}

impl IntoFuture for ListRequest {
    type Output = Result<Vec<ListEntry>, Error>;
    type IntoFuture = crate::RequestFuture<Vec<ListEntry>>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            let message = if self.channels.is_empty() {
                Line::new("LIST")
            } else {
                Line::new("LIST").with_params([self.channels.join(",")])
            };
            let response = self
                .client
                .correlated(message, Some(ResponseMatcher::List))
                .await?;
            list(
                response,
                self.client.state().isupport().inner(),
                self.client.origin(),
            )
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
) -> Result<Option<Whois>, Error> {
    whole(&response)?;
    if let Some(error) = response.error() {
        if error.refusal() == crate::Refusal::NoSuchNick {
            return Ok(None);
        }
        return Err(Error::server(error));
    }
    let mapping = isupport.casemapping();
    let mut result = Whois {
        nick: origin.__nick(mapping, requested_nick),
        username: None,
        host: None,
        realname: None,
        account: Account::Unknown,
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
    Ok(Some(result))
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
    client: &crate::Client,
    extended: bool,
) -> Result<Vec<WhoEntry>, Error> {
    empty(&response)?;
    let origin = client.origin();
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
                    channel: reply.channel.map(|channel| {
                        crate::ChannelHandle::new(client, origin.__channel(mapping, channel), false)
                    }),
                    user: crate::UserHandle::from_current_state(
                        origin.__nick(mapping, reply.nick?),
                        Some(&client.state),
                    ),
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
                    is_oper: flags.oper,
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
                plain_who(&message, isupport, client)
            }
        })
        .collect())
}

fn plain_who(
    message: &papo_proto::LineRef<'_>,
    isupport: &Isupport,
    client: &crate::Client,
) -> Option<WhoEntry> {
    (message.command() == Command::Numeric(Numeric::RPL_WHOREPLY)).then_some(())?;
    let flags = message.param(6)?;
    let prefixes = isupport.prefix();
    let origin = client.origin();
    Some(WhoEntry {
        channel: message
            .param(1)
            .filter(|value| *value != "*")
            .map(|channel| {
                crate::ChannelHandle::new(
                    client,
                    origin.__channel(isupport.casemapping(), channel),
                    false,
                )
            }),
        username: message.param(2)?.to_owned(),
        host: message.param(3)?.to_owned(),
        user: crate::UserHandle::from_current_state(
            origin.__nick(isupport.casemapping(), message.param(5)?),
            Some(&client.state),
        ),
        account: Account::Unknown,
        realname: message.param(7)?.split_once(' ')?.1.to_owned(),
        away: if flags.contains('G') {
            Away::__away_value(None)
        } else if flags.contains('H') {
            Away::Present
        } else {
            Away::Unknown
        },
        is_oper: flags.contains('*'),
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

/// Refuses a reply that a state bound cut short, so a partial answer never looks complete.
pub(crate) fn whole(response: &Response) -> Result<(), Error> {
    response
        .state_limit
        .map_or(Ok(()), |limit| Err(limit.into()))
}

pub(crate) fn empty(response: &Response) -> Result<(), Error> {
    whole(response)?;
    response
        .error()
        .map_or(Ok(()), |error| Err(Error::server(error)))
}

/// Returns our command's echo in `response`, which confirms the request.
///
/// A response that ends without the echo, such as a bare `ACK`, did not change anything papo can
/// see, so it is [`Error::NotVerified`].
pub(crate) fn echo<'a>(response: &'a Response, command: &str) -> Result<LineRef<'a>, Error> {
    empty(response)?;
    response
        .lines
        .iter()
        .map(Line::as_ref)
        .find(|line| line.command_is(command))
        .ok_or(Error::NotVerified)
}

pub(crate) fn list(
    response: Response,
    isupport: &Isupport,
    origin: crate::Origin,
) -> Result<Vec<ListEntry>, Error> {
    empty(&response)?;
    Ok(list_entries(response.lines, isupport, origin))
}

pub(crate) fn list_mode(
    response: Response,
    origin: crate::Origin,
) -> Result<Vec<crate::ModeListEntry>, Error> {
    empty(&response)?;
    response
        .mode_list
        .map(|entries| crate::state::mode_list_from_core(&entries, origin))
        .ok_or_else(|| Error::unexpected_reply("MODE"))
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
        .collect()
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::Instant;

    use papo_core::{Account, Away, Config, Event, ResponseMatcher, Session};
    use papo_proto::{Casemapping, Isupport, Line};

    use super::{
        Response, WHOX_FIELDS, empty, list, list_entries, list_mode, plain_who, who, whois,
        whois_channel,
    };
    use crate::Error;

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
    fn whois_only_reports_an_account_fact_proved_by_the_server() {
        let reply = |lines: &[&str]| {
            let now = Instant::now();
            let mut session = Session::new(Config::default().nick("papo"));
            session
                .request_with_matcher(
                    now,
                    Line::new("WHOIS").with_params(["alice"]),
                    ResponseMatcher::Whois("alice".into()),
                )
                .unwrap();
            for line in lines {
                session.handle_line(now, line.as_bytes());
            }
            let response = std::iter::from_fn(|| session.poll_event())
                .find_map(|event| match event {
                    Event::Response(response) => Some(response),
                    _ => None,
                })
                .unwrap();
            whois(response, "alice", &Isupport::default(), crate::Origin::NONE)
                .unwrap()
                .unwrap()
                .account
        };

        assert!(matches!(
            reply(&[
                ":ergo.test 311 papo alice ~alice client.example * :Alice Example",
                ":ergo.test 330 papo alice alice :is logged in as",
                ":ergo.test 318 papo alice :End of /WHOIS list",
            ]),
            Account::LoggedIn(account) if account == "alice"
        ));
        assert_eq!(
            reply(&[
                ":irc.papo.test 311 papo alice ~alice users.example * :Alice Example",
                ":irc.papo.test 312 papo alice irc.papo.test :papo test server",
                ":irc.papo.test 318 papo alice :End of /WHOIS list",
            ]),
            Account::Unknown
        );
    }

    #[test]
    fn who_entry_is_oper_for_plain_and_whox() {
        let isupport = Isupport::default();
        let client = crate::Client::disconnected_for_test();
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

        let hidden = plain_who(&hidden.as_ref(), &isupport, &client).unwrap();
        let visible = plain_who(&visible.as_ref(), &isupport, &client).unwrap();
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
        assert!(hidden.is_oper);
        assert_eq!(hidden.rank(), Some(crate::Rank::Op));
        assert_eq!(
            (
                visible.channel.as_ref().map(crate::ChannelHandle::name),
                visible.prefixes[0].symbol()
            ),
            (Some(client.channel("#rust").unwrap().name()), '+')
        );
        assert_eq!(visible.rank(), Some(crate::Rank::Voice));

        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session
            .request_with_matcher(
                now,
                Line::new("WHO").with_params(["#rust", WHOX_FIELDS]),
                ResponseMatcher::Who("#rust".into()),
            )
            .unwrap();
        for line in [
            ":ergo.test 354 papo 152 #rust ~alice staff.example alice H*@ alice :Alice Example",
            ":ergo.test 315 papo #rust :End of WHO list",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let response = std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) => Some(response),
                _ => None,
            })
            .unwrap();
        let extended = who(response, &isupport, &client, true).unwrap();
        assert_eq!(extended.len(), 1);
        assert!(extended[0].is_oper);
        assert_eq!(extended[0].user.nick(), "alice");
    }

    struct BoundedCase {
        request: Line,
        matcher: ResponseMatcher,
        labelled: bool,
        head: &'static [&'static str],
        row: &'static str,
        end: &'static str,
        bound: usize,
        build: fn(Response, &crate::Client) -> Result<(), Error>,
    }

    fn bounded_session(now: Instant, labelled: bool) -> Session {
        let mut session = Session::new(
            Config::default()
                .nick("papo")
                .rate_limit(papo_core::RateLimit::unlimited()),
        );
        if labelled {
            session.handle_connected(now);
            for line in [
                "CAP * LS :labeled-response",
                "CAP * ACK :labeled-response",
                ":ergo.test 001 papo :Welcome to the Ergo test network",
                ":ergo.test 422 papo :MOTD File is missing",
            ] {
                session.handle_line(now, line.as_bytes());
            }
            while session.poll_transmit().is_some() {}
            while session.poll_event().is_some() {}
        }
        session
    }

    fn bounded_reply(case: &BoundedCase, rows: usize) -> Response {
        let now = Instant::now();
        let mut session = bounded_session(now, case.labelled);
        session
            .request_with_matcher(now, case.request.clone(), case.matcher.clone())
            .unwrap();
        let rows = (0..rows).map(|index| case.row.replace("{i}", &index.to_string()));
        let mut lines: Vec<String> = case.head.iter().map(|line| (*line).to_owned()).collect();
        lines.extend(rows);
        lines.push(case.end.to_owned());
        if case.labelled {
            let sent = format!("{:?}", session.poll_transmit().unwrap());
            let label = sent
                .split("@label=")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap();
            lines = std::iter::once(format!(
                "@label={label} :ergo.test BATCH +reply labeled-response"
            ))
            .chain(lines.into_iter().map(|line| format!("@batch=reply {line}")))
            .chain([":ergo.test BATCH -reply".to_owned()])
            .collect();
        }
        for line in &lines {
            session.handle_line(now, line.as_bytes());
        }
        std::iter::from_fn(|| session.poll_event())
            .find_map(|event| match event {
                Event::Response(response) => Some(response),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn every_bounded_reply_refuses_a_truncated_answer() {
        let session = Session::new(Config::default().nick("papo"));
        let client = crate::driver::test_support::client_with_commands(&session).0;
        let cases = [
            BoundedCase {
                request: Line::new("WHO").with_params(["#lobby"]),
                matcher: ResponseMatcher::Who("#lobby".into()),
                labelled: false,
                head: &[],
                row: ":ergo.test 352 papo #lobby ~guest guest-{i}.example ergo.test guest{i} H :0 Guest {i}",
                end: ":ergo.test 315 papo #lobby :End of WHO list",
                bound: 10_000,
                build: |response, client| {
                    who(response, &Isupport::default(), client, false).map(drop)
                },
            },
            BoundedCase {
                request: Line::new("WHO").with_params(["#lobby", WHOX_FIELDS]),
                matcher: ResponseMatcher::Who("#lobby".into()),
                labelled: false,
                head: &[],
                row: ":ergo.test 354 papo 152 #lobby ~guest guest-{i}.example guest{i} H 0 :Guest {i}",
                end: ":ergo.test 315 papo #lobby :End of WHO list",
                bound: 10_000,
                build: |response, client| {
                    who(response, &Isupport::default(), client, true).map(drop)
                },
            },
            BoundedCase {
                request: Line::new("LIST"),
                matcher: ResponseMatcher::List,
                labelled: false,
                head: &[":ergo.test 321 papo Channel :Users Name"],
                row: ":ergo.test 322 papo #channel-{i} 3 :[+nt] chat",
                end: ":ergo.test 323 papo :End of /LIST",
                bound: 10_000,
                build: |response, _| {
                    list(response, &Isupport::default(), crate::Origin::NONE).map(drop)
                },
            },
            BoundedCase {
                request: Line::new("WHOIS").with_params(["alice"]),
                matcher: ResponseMatcher::Whois("alice".into()),
                labelled: false,
                head: &[],
                row: ":ergo.test 319 papo alice :#channel-{i}",
                end: ":ergo.test 318 papo alice :End of /WHOIS list",
                bound: 10_000,
                build: |response, _| {
                    whois(response, "alice", &Isupport::default(), crate::Origin::NONE).map(drop)
                },
            },
            BoundedCase {
                request: Line::new("WHO").with_params(["guest1", "%na"]),
                matcher: ResponseMatcher::Who("guest1".into()),
                labelled: false,
                head: &[],
                row: ":ergo.test 354 papo guest{i} 0",
                end: ":ergo.test 315 papo guest1 :End of WHO list",
                bound: 10_000,
                build: |response, client| {
                    crate::identity::who_account(client, "guest1", &response).map(drop)
                },
            },
            BoundedCase {
                request: Line::new("MODE").with_params(["#papo", "b"]),
                matcher: ResponseMatcher::__list_mode("#papo".into(), crate::ListMode::Ban),
                labelled: false,
                head: &[],
                row: ":ergo.test 367 papo #papo *!*@host{i}.example oper 1761048000",
                end: ":ergo.test 368 papo #papo :End of channel ban list",
                bound: 1_000,
                build: |response, _| list_mode(response, crate::Origin::NONE).map(drop),
            },
            BoundedCase {
                request: Line::new("WHO").with_params(["#lobby"]),
                matcher: ResponseMatcher::Who("#lobby".into()),
                labelled: true,
                head: &[],
                row: ":ergo.test 352 papo #lobby ~guest guest-{i}.example ergo.test guest{i} H :0 Guest {i}",
                end: ":ergo.test 315 papo #lobby :End of WHO list",
                bound: 9_999,
                build: |response, client| {
                    who(response, &Isupport::default(), client, false).map(drop)
                },
            },
        ];

        for case in &cases {
            let request = case.request.to_string();
            assert!(
                matches!(
                    (case.build)(bounded_reply(case, case.bound + 1), &client),
                    Err(Error::StateLimit(papo_core::StateLimit::ListEntries))
                ),
                "{request} accepted a truncated reply"
            );
            assert!(
                (case.build)(bounded_reply(case, case.bound), &client).is_ok(),
                "{request} refused a reply at the bound"
            );
        }
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
    fn list_retains_every_bounded_core_entry() {
        let lines = (0..10_000).map(|index| {
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
        let session = Session::new(Config::default().casemapping(Casemapping::Ascii));
        let client = crate::driver::test_support::client_with_commands(&session).0;
        let origin = client.origin();
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
        let who = plain_who(&who.as_ref(), &isupport, &client).unwrap();

        assert_eq!(channel.name.casemapping(), Casemapping::Ascii);
        assert_eq!(
            who.channel.unwrap().name().casemapping(),
            Casemapping::Ascii
        );
        assert_eq!(who.user.nick().casemapping(), Casemapping::Ascii);
        assert_eq!(channel.name.origin(), origin);
        assert_eq!(who.user.nick().origin(), origin);
    }
}
