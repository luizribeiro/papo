use std::time::Duration;

use papo_core::Response;
use papo_proto::{Command, Numeric};

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
