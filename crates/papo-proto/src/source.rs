/// The source of an IRC message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source<'a> {
    /// A server name.
    Server(&'a str),
    /// A user, split into nickname, username, and hostname components.
    User {
        /// The user's nickname.
        nick: &'a str,
        /// The username, when supplied by the server.
        user: Option<&'a str>,
        /// The hostname, when supplied by the server.
        host: Option<&'a str>,
    },
}

impl<'a> Source<'a> {
    pub(crate) fn parse(source: &'a str) -> Self {
        if !source.contains(['!', '@']) && source.contains('.') {
            return Self::Server(source);
        }

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
        Self::User {
            nick,
            user: user.filter(|user| !user.is_empty()),
            host: host.filter(|host| !host.is_empty()),
        }
    }
}
