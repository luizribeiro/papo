use std::time::Duration;

use papo_core::Response;
use papo_proto::{Command, Isupport, Message, Numeric, WhoxPresence, WhoxReply};

use crate::Error;

/// Information returned by a `WHOIS` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Whois {
    /// Nickname reported by the server.
    pub nick: String,
    /// Username reported by `RPL_WHOISUSER`.
    pub user: Option<String>,
    /// Hostname reported by `RPL_WHOISUSER`.
    pub host: Option<String>,
    /// Real name reported by `RPL_WHOISUSER`.
    pub realname: Option<String>,
    /// Logged-in account name.
    pub account: Option<String>,
    /// Server to which the user is connected.
    pub server: Option<String>,
    /// Visible channels, retaining membership prefixes.
    pub channels: Vec<String>,
    /// Away message, when the user is away.
    pub away: Option<String>,
    /// Whether the server identifies the user as a bot.
    pub is_bot: bool,
    /// Whether the server identifies the connection as secure.
    pub secure: bool,
    /// Duration for which the user has been idle.
    pub idle: Option<Duration>,
}

/// One user returned by a `WHO` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhoEntry {
    /// Channel associated with this reply, when present.
    pub channel: Option<String>,
    /// Nickname of the user.
    pub nick: String,
    /// Username of the user.
    pub user: String,
    /// Visible hostname of the user.
    pub host: String,
    /// Logged-in account name.
    pub account: Option<String>,
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

/// State established by a completed channel join.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelInfo {
    /// Channel name as reported by the server.
    pub name: String,
    /// Current channel topic, when one is set.
    pub topic: Option<String>,
    /// Number of members present after initial synchronization.
    pub members_count: usize,
}

/// Optional channel names or masks supplied to `LIST`.
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
        S: Into<String>,
    {
        self.channels = channels.into_iter().map(Into::into).collect();
        self
    }

    pub(crate) fn parameter(&self) -> Option<String> {
        (!self.channels.is_empty()).then(|| self.channels.join(","))
    }
}

/// One channel returned by `LIST`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListEntry {
    /// Channel name.
    pub name: String,
    /// Number of visible members reported by the server.
    pub members_count: usize,
    /// Channel topic.
    pub topic: String,
}

pub(crate) fn whois(response: Response, requested_nick: &str) -> Result<Whois, Error> {
    if let Some(error) = response.error() {
        return Err(Error::Server(error));
    }
    let mut result = Whois {
        nick: requested_nick.to_owned(),
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
                message
                    .param(1)
                    .unwrap_or(requested_nick)
                    .clone_into(&mut result.nick);
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
                    .map(str::to_owned),
            ),
            Numeric::RPL_WHOISACCOUNT => result.account = message.param(2).map(str::to_owned),
            Numeric::RPL_AWAY => result.away = message.param(2).map(str::to_owned),
            Numeric::RPL_WHOISBOT => result.is_bot = true,
            Numeric::RPL_WHOISSECURE => result.secure = true,
            _ => {}
        }
    }
    Ok(result)
}

pub(crate) fn who(
    response: Response,
    isupport: &Isupport,
    extended: bool,
) -> Result<Vec<WhoEntry>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::Server(error));
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
                    channel: reply.channel.map(str::to_owned),
                    nick: reply.nick?.to_owned(),
                    user: reply.user?.to_owned(),
                    host: reply.host?.to_owned(),
                    account: reply.account.map(str::to_owned),
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
            .map(str::to_owned),
        user: message.param(2)?.to_owned(),
        host: message.param(3)?.to_owned(),
        nick: message.param(5)?.to_owned(),
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
        .map_or(Ok(()), |error| Err(Error::Server(error)))
}

pub(crate) fn topic(response: &Response) -> Result<Option<String>, Error> {
    if let Some(error) = response.error() {
        return Err(Error::Server(error));
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
        return Err(Error::Server(error));
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
                name: message.param(1)?.to_owned(),
                members_count: message.param(2)?.parse().ok()?,
                topic: message.param(3)?.to_owned(),
            })
        })
        .take(10_000)
        .collect()
}

#[cfg(test)]
mod tests {
    use papo_proto::Message;

    use super::list_entries;

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
