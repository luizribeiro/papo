/// The source of an IRC line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source<'a> {
    /// A server name.
    Server(&'a str),
    /// A user, split into nickname, username, and hostname components.
    User {
        /// The user's nickname.
        nick: &'a str,
        /// The username from `nick!user@host`, when supplied by the server.
        username: Option<&'a str>,
        /// The hostname, when supplied by the server.
        host: Option<&'a str>,
    },
}

impl<'a> Source<'a> {
    pub(crate) fn parse(source: &'a str) -> Self {
        if !source.contains(['!', '@']) && source.contains('.') {
            return Self::Server(source);
        }

        let (nick, username, host) = split_user(source);
        Self::User {
            nick,
            username,
            host,
        }
    }
}

pub(crate) fn split_user(source: &str) -> (&str, Option<&str>, Option<&str>) {
    let (nick, user, host) = if let Some((nick, remainder)) = source.split_once('!') {
        let (user, host) = remainder
            .split_once('@')
            .map_or((Some(remainder), None), |(user, host)| {
                (Some(user), Some(host))
            });
        (nick, user, host)
    } else if let Some((nick, host)) = source.split_once('@') {
        (nick, None, Some(host))
    } else {
        (source, None, None)
    };
    (
        nick,
        user.filter(|user| !user.is_empty()),
        host.filter(|host| !host.is_empty()),
    )
}
