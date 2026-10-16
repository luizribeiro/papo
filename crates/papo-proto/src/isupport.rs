use std::{borrow::Cow, collections::HashMap};

use crate::{Casemapping, Command, MessageRef, Numeric, canonical};

/// Whether an ISUPPORT token is absent, bare, or carries a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Advertised<'a> {
    /// The server has not advertised the token.
    Absent,
    /// The server advertised `KEY` without an equals sign.
    Bare,
    /// The server advertised `KEY=VALUE`, including a possibly empty value.
    Value(&'a str),
}

/// One ranked channel-membership mode and its visible prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Prefix {
    /// The membership mode letter.
    pub mode: char,
    /// The symbol shown before a nickname.
    pub symbol: char,
    /// The zero-based rank, with zero being the highest privilege.
    pub rank: usize,
}

/// The four standard channel-mode parameter groups.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChanModes<'a> {
    /// List modes that always take a parameter.
    pub list: &'a str,
    /// Non-list modes that always take a parameter.
    pub always: &'a str,
    /// Modes taking a parameter only when set.
    pub when_set: &'a str,
    /// Modes that never take a parameter.
    pub never: &'a str,
}

/// A named limit, where `None` means unlimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limit<'a> {
    /// The command, channel prefixes, or mode letters governed by the limit.
    ///
    /// Command names returned by [`Isupport::targmax`] compare case-insensitively.
    pub name: &'a str,
    /// The advertised maximum, or `None` when no maximum applies.
    pub limit: Option<usize>,
}

/// A numeric server limit that may explicitly be unlimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitValue {
    /// No maximum applies.
    Unlimited,
    /// The advertised maximum.
    Limited(usize),
}

/// The prefix and type letters used for extended bans.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Extban<'a> {
    /// The optional character introducing an extended ban.
    pub prefix: Option<char>,
    /// The supported extended-ban type letters.
    pub types: &'a str,
}

/// A parsed `CLIENTTAGDENY` policy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClientTagDeny<'a> {
    deny_all: bool,
    blocked: Vec<&'a str>,
    allowed: Vec<&'a str>,
}

impl ClientTagDeny<'_> {
    /// Reports whether the named client-only tag is blocked.
    #[must_use]
    pub fn denies(&self, tag: &str) -> bool {
        let tag = tag.strip_prefix('+').unwrap_or(tag);
        if self.deny_all {
            !self.allowed.contains(&tag)
        } else {
            self.blocked.contains(&tag)
        }
    }
}

/// Accumulated server features advertised through `RPL_ISUPPORT` (`005`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Isupport {
    tokens: HashMap<String, Option<String>>,
}

macro_rules! length_accessors {
    ($(#[$meta:meta] $name:ident => $key:literal,)+) => {$(
        #[$meta]
        #[must_use]
        pub fn $name(&self) -> Option<usize> {
            positive(self.value($key)?)
        }
    )+};
}

macro_rules! flag_accessors {
    ($(#[$meta:meta] $name:ident => $key:literal,)+) => {$(
        #[$meta]
        #[must_use]
        pub fn $name(&self) -> bool {
            !matches!(self.get($key), Advertised::Absent)
        }
    )+};
}

impl Isupport {
    /// Applies the tokens in an `RPL_ISUPPORT` message.
    ///
    /// Other commands, including `RPL_REMOTEISUPPORT` (`105`), are ignored.
    pub fn apply(&mut self, message: &MessageRef<'_>) {
        if message.command() != Command::Numeric(Numeric::RPL_ISUPPORT) {
            return;
        }
        let mut params = message.params();
        params.next();
        let Some(mut pending) = params.next() else {
            return;
        };
        // Server-specific trailer prose must not become unstable feature-state keys.
        for next in params {
            self.apply_tokens([pending]);
            pending = next;
        }
    }

    /// Applies `KEY`, `KEY=VALUE`, and `-KEY` tokens in order.
    pub fn apply_tokens<I, S>(&mut self, tokens: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for token in tokens {
            let token = token.as_ref();
            if let Some(key) = token.strip_prefix('-') {
                self.tokens.remove(&canonical(key).to_ascii_uppercase());
            } else if let Some((key, value)) = token.split_once('=') {
                if !key.is_empty() {
                    self.tokens
                        .insert(canonical(key).to_ascii_uppercase(), Some(unescape(value)));
                }
            } else if !token.is_empty() {
                self.tokens
                    .insert(canonical(token).to_ascii_uppercase(), None);
            }
        }
    }

    /// Returns whether a token is absent, bare, or has a value.
    #[must_use]
    pub fn get(&self, key: &str) -> Advertised<'_> {
        match self.tokens.get(&canonical(key).to_ascii_uppercase()) {
            None => Advertised::Absent,
            Some(None) => Advertised::Bare,
            Some(Some(value)) => Advertised::Value(value),
        }
    }

    /// Returns the server's name casemapping.
    #[must_use]
    pub fn casemapping(&self) -> Casemapping {
        #[cfg(feature = "precis")]
        if self.value("UTF8MAPPING") == Some("rfc8265") {
            return Casemapping::Precis;
        }
        self.value("CASEMAPPING")
            .and_then(Casemapping::parse)
            .unwrap_or_default()
    }

    /// Returns the channel-type prefix characters.
    #[must_use]
    pub fn chantypes(&self) -> &str {
        match self.get("CHANTYPES") {
            Advertised::Absent => "#&",
            Advertised::Bare => "",
            Advertised::Value(value) => value,
        }
    }

    /// Returns channel membership prefixes from highest to lowest rank.
    #[must_use]
    pub fn prefix(&self) -> Vec<Prefix> {
        parse_prefix(self.value("PREFIX").unwrap_or("(ov)@+"))
            .or_else(|| parse_prefix("(ov)@+"))
            .unwrap_or_default()
    }

    /// Returns the four standard `CHANMODES` groups, ignoring any later groups.
    #[must_use]
    pub fn chanmodes(&self) -> Option<ChanModes<'_>> {
        let mut groups = self.value("CHANMODES")?.split(',');
        Some(ChanModes {
            list: groups.next()?,
            always: groups.next()?,
            when_set: groups.next()?,
            never: groups.next()?,
        })
    }

    /// Returns the prefixes accepted before channel message targets.
    #[must_use]
    pub fn statusmsg(&self) -> Option<&str> {
        self.value("STATUSMSG")
    }

    /// Reports whether a target starts with an advertised channel-type prefix.
    #[must_use]
    pub fn is_channel(&self, name: &str) -> bool {
        name.chars()
            .next()
            .is_some_and(|prefix| self.chantypes().contains(prefix))
    }

    /// Returns per-command target limits, defaulting to unlimited `JOIN` and `PART`.
    ///
    /// Command names compare case-insensitively.
    #[must_use]
    pub fn targmax(&self) -> Vec<Limit<'_>> {
        self.value("TARGMAX")
            .and_then(|value| parse_limits(value, true))
            .unwrap_or_else(default_targmax)
    }

    /// Returns the legacy shared target limit.
    #[must_use]
    pub fn maxtargets(&self) -> Option<LimitValue> {
        optional_limit(self.get("MAXTARGETS"), false)
    }

    /// Returns the variable-mode limit, where `None` means unlimited.
    #[must_use]
    pub fn modes(&self) -> Option<usize> {
        match self.get("MODES") {
            Advertised::Absent => Some(3),
            Advertised::Bare | Advertised::Value("") => None,
            Advertised::Value(value) => positive(value).or(Some(3)),
        }
    }

    /// Returns limits grouped by channel-type prefixes.
    #[must_use]
    pub fn chanlimit(&self) -> Vec<Limit<'_>> {
        self.value("CHANLIMIT")
            .and_then(|value| parse_limits(value, true))
            .unwrap_or_default()
    }

    /// Returns limits grouped by list-mode letters.
    #[must_use]
    pub fn maxlist(&self) -> Vec<Limit<'_>> {
        self.value("MAXLIST")
            .and_then(|value| parse_limits(value, false))
            .unwrap_or_default()
    }

    length_accessors! {
        /// Returns the maximum nickname length.
        nicklen => "NICKLEN",
        /// Returns the maximum channel-name length.
        channellen => "CHANNELLEN",
        /// Returns the maximum topic length.
        topiclen => "TOPICLEN",
        /// Returns the maximum kick-reason length.
        kicklen => "KICKLEN",
        /// Returns the maximum away-reason length.
        awaylen => "AWAYLEN",
        /// Returns the maximum real-name length.
        namelen => "NAMELEN",
    }

    /// Returns the advertised network name.
    #[must_use]
    pub fn network(&self) -> Option<&str> {
        self.value("NETWORK")
    }

    /// Returns the network's advertised icon URL or URL template.
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.value("DRAFT/ICON")
    }

    /// Returns the network icon URL with every `{size}` placeholder filled.
    ///
    /// The requested size is a hint to the image server.
    #[must_use]
    pub fn icon_url(&self, size: u32) -> Option<Cow<'_, str>> {
        let icon = self.icon()?;
        if icon.contains("{size}") {
            Some(Cow::Owned(icon.replace("{size}", &size.to_string())))
        } else {
            Some(Cow::Borrowed(icon))
        }
    }

    flag_accessors! {
        /// Reports whether the server accepts only UTF-8 messages.
        utf8only => "UTF8ONLY",
        /// Reports whether extended `WHO` syntax is available.
        whox => "WHOX",
        /// Reports whether authentication is required.
        account_required => "DRAFT/ACCOUNTREQUIRED",
        /// Reports whether safe `LIST` behavior is available.
        safelist => "SAFELIST",
    }

    /// Returns the user mode used to identify bots.
    #[must_use]
    pub fn bot_mode(&self) -> Option<char> {
        one_char(self.value("BOT")?)
    }

    /// Returns the monitor-list limit.
    #[must_use]
    pub fn monitor(&self) -> Option<LimitValue> {
        optional_limit(self.get("MONITOR"), true)
    }

    /// Returns the history request limit.
    #[must_use]
    pub fn chathistory(&self) -> Option<LimitValue> {
        optional_limit(self.get("CHATHISTORY"), true)
    }

    /// Returns supported message-reference types in preference order.
    ///
    /// When the token is absent, the specification states no preference; the
    /// returned default uses Ergo's `msgid, timestamp` order.
    #[must_use]
    pub fn msgreftypes(&self) -> Vec<&str> {
        self.value("MSGREFTYPES").map_or_else(
            || vec!["msgid", "timestamp"],
            |value| value.split(',').filter(|part| !part.is_empty()).collect(),
        )
    }

    /// Returns the server's client-only tag filtering policy.
    #[must_use]
    pub fn clienttagdeny(&self) -> ClientTagDeny<'_> {
        let Some(value) = self.value("CLIENTTAGDENY") else {
            return ClientTagDeny::default();
        };
        let mut policy = ClientTagDeny::default();
        for item in value.split(',').filter(|item| !item.is_empty()) {
            if item == "*" {
                policy.deny_all = true;
            } else if let Some(tag) = item.strip_prefix('-') {
                policy.allowed.push(tag);
            } else {
                policy.blocked.push(item);
            }
        }
        policy
    }

    /// Returns the ban-exception mode, using `e` for a bare token.
    #[must_use]
    pub fn excepts(&self) -> Option<char> {
        optional_mode(self.get("EXCEPTS"), 'e')
    }

    /// Returns the invite-exception mode, using `I` for a bare token.
    #[must_use]
    pub fn invex(&self) -> Option<char> {
        optional_mode(self.get("INVEX"), 'I')
    }

    /// Returns the extended-ban prefix and supported types.
    #[must_use]
    pub fn extban(&self) -> Option<Extban<'_>> {
        let (prefix, types) = self.value("EXTBAN")?.split_once(',')?;
        Some(Extban {
            prefix: if prefix.is_empty() {
                None
            } else {
                Some(one_char(prefix)?)
            },
            types,
        })
    }

    /// Returns extended-ban names that refer to accounts.
    #[must_use]
    pub fn accountextban(&self) -> Vec<&str> {
        self.value("ACCOUNTEXTBAN")
            .map(|value| value.split(',').filter(|part| !part.is_empty()).collect())
            .unwrap_or_default()
    }

    /// Returns the case-insensitive `LIST` search extension letters.
    #[must_use]
    pub fn elist(&self) -> Option<&str> {
        self.value("ELIST")
    }

    fn value(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Advertised::Value(value) => Some(value),
            Advertised::Absent | Advertised::Bare => None,
        }
    }
}

fn parse_prefix(value: &str) -> Option<Vec<Prefix>> {
    if value.is_empty() {
        return Some(Vec::new());
    }
    let (modes, symbols) = value.strip_prefix('(')?.split_once(')')?;
    if modes.chars().count() != symbols.chars().count() {
        return None;
    }
    Some(
        modes
            .chars()
            .zip(symbols.chars())
            .enumerate()
            .map(|(rank, (mode, symbol))| Prefix { mode, symbol, rank })
            .collect(),
    )
}

fn parse_limits(value: &str, empty_is_unlimited: bool) -> Option<Vec<Limit<'_>>> {
    if value.is_empty() {
        return None;
    }
    value
        .split(',')
        .map(|pair| {
            let (name, value) = pair.split_once(':')?;
            if name.is_empty() {
                return None;
            }
            let limit = if value.is_empty() && empty_is_unlimited {
                None
            } else {
                Some(positive(value)?)
            };
            Some(Limit { name, limit })
        })
        .collect()
}

fn default_targmax<'a>() -> Vec<Limit<'a>> {
    vec![
        Limit {
            name: "JOIN",
            limit: None,
        },
        Limit {
            name: "PART",
            limit: None,
        },
    ]
}

fn positive(value: &str) -> Option<usize> {
    value.parse().ok().filter(|value| *value > 0)
}

fn optional_limit(value: Advertised<'_>, zero_is_unlimited: bool) -> Option<LimitValue> {
    match value {
        Advertised::Absent => None,
        Advertised::Bare | Advertised::Value("") => Some(LimitValue::Unlimited),
        Advertised::Value("0") if zero_is_unlimited => Some(LimitValue::Unlimited),
        Advertised::Value(value) => positive(value).map(LimitValue::Limited),
    }
}

fn optional_mode(value: Advertised<'_>, default: char) -> Option<char> {
    match value {
        Advertised::Absent => None,
        Advertised::Bare | Advertised::Value("") => Some(default),
        Advertised::Value(value) => one_char(value),
    }
}

fn one_char(value: &str) -> Option<char> {
    let mut chars = value.chars();
    let character = chars.next()?;
    chars.next().is_none().then_some(character)
}

fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut rest = value;
    while !rest.is_empty() {
        let (replacement, consumed) = if rest.starts_with("\\x20") {
            (' ', 4)
        } else if rest.starts_with("\\x5C") {
            ('\\', 4)
        } else if rest.starts_with("\\x3D") {
            ('=', 4)
        } else {
            let Some(character) = rest.chars().next() else {
                break;
            };
            (character, character.len_utf8())
        };
        result.push(replacement);
        rest = &rest[consumed..];
    }
    result
}
