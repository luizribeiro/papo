use papo_proto::{Advertised, ChanModes, ClientTagDeny, ExtbanSyntax, Limit, LimitValue};

use std::borrow::Cow;

use crate::Casemapping;

/// Accumulated server features advertised through `RPL_ISUPPORT` (`005`).
///
/// This facade intentionally provides no way to apply server messages. A value returned by
/// [`crate::State::isupport`] can only describe the active connection's reported features.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Isupport {
    inner: papo_proto::Isupport,
}

impl Isupport {
    pub(crate) const fn from_proto(inner: papo_proto::Isupport) -> Self {
        Self { inner }
    }

    pub(crate) const fn inner(&self) -> &papo_proto::Isupport {
        &self.inner
    }

    /// Returns whether a token is absent, bare, or has a value.
    #[must_use]
    pub fn get(&self, key: &str) -> Advertised<'_> {
        self.inner.get(key)
    }

    /// Iterates over every advertised token in unspecified order.
    #[must_use]
    pub fn tokens(&self) -> impl ExactSizeIterator<Item = (&str, Advertised<'_>)> {
        self.inner.tokens()
    }

    /// Returns the server's name casemapping.
    #[must_use]
    pub fn casemapping(&self) -> Option<Casemapping> {
        self.inner.__received().then(|| self.inner.casemapping())
    }

    /// Returns the channel-type prefix characters.
    #[must_use]
    pub fn chantypes(&self) -> &str {
        self.inner.chantypes()
    }

    /// Returns channel membership prefixes from highest to lowest rank.
    #[must_use]
    pub fn prefix(&self) -> Vec<crate::Prefix> {
        self.inner.prefix()
    }

    /// Returns the four standard `CHANMODES` groups, ignoring any later groups.
    #[must_use]
    pub fn chanmodes(&self) -> Option<ChanModes<'_>> {
        self.inner.chanmodes()
    }

    /// Returns the prefixes accepted before channel message targets.
    #[must_use]
    pub fn statusmsg(&self) -> Option<&str> {
        self.inner.statusmsg()
    }

    /// Reports whether a target has an advertised channel prefix.
    #[must_use]
    pub fn is_channel(&self, name: &str) -> bool {
        self.inner.is_channel(name)
    }

    /// Returns per-command target limits, defaulting to unlimited `JOIN` and `PART`.
    #[must_use]
    pub fn targmax(&self) -> Vec<Limit<'_>> {
        self.inner.targmax()
    }

    /// Returns the target limit for `command`.
    ///
    /// Command names compare case-insensitively.
    #[must_use]
    pub fn targmax_for(&self, command: &str) -> Option<LimitValue> {
        self.inner.targmax_for(command)
    }

    /// Returns the legacy shared target limit.
    #[must_use]
    pub fn maxtargets(&self) -> Option<LimitValue> {
        self.inner.maxtargets()
    }

    /// Returns the variable-mode limit, where `None` means unlimited.
    #[must_use]
    pub fn modes(&self) -> Option<usize> {
        self.inner.modes()
    }

    /// Returns limits grouped by channel-type prefixes.
    #[must_use]
    pub fn chanlimit(&self) -> Vec<Limit<'_>> {
        self.inner.chanlimit()
    }

    /// Returns limits grouped by list-mode letters.
    #[must_use]
    pub fn maxlist(&self) -> Vec<Limit<'_>> {
        self.inner.maxlist()
    }

    /// Returns the maximum nickname length.
    #[must_use]
    pub fn nicklen(&self) -> Option<usize> {
        self.inner.nicklen()
    }

    /// Returns the maximum channel-name length.
    #[must_use]
    pub fn channellen(&self) -> Option<usize> {
        self.inner.channellen()
    }

    /// Returns the maximum topic length.
    #[must_use]
    pub fn topiclen(&self) -> Option<usize> {
        self.inner.topiclen()
    }

    /// Returns the maximum kick-reason length.
    #[must_use]
    pub fn kicklen(&self) -> Option<usize> {
        self.inner.kicklen()
    }

    /// Returns the maximum away-reason length.
    #[must_use]
    pub fn awaylen(&self) -> Option<usize> {
        self.inner.awaylen()
    }

    /// Returns the maximum real-name length.
    #[must_use]
    pub fn namelen(&self) -> Option<usize> {
        self.inner.namelen()
    }

    /// Returns the advertised network name.
    #[must_use]
    pub fn network(&self) -> Option<&str> {
        self.inner.network()
    }

    /// Returns the network's advertised icon URL or URL template.
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.inner.icon()
    }

    /// Returns the network icon URL with every `{size}` placeholder filled.
    #[must_use]
    pub fn icon_url(&self, size: u32) -> Option<Cow<'_, str>> {
        self.inner.icon_url(size)
    }

    /// Returns the server's Web Push VAPID public key.
    #[cfg(feature = "unstable")]
    #[must_use]
    pub fn vapid(&self) -> Option<&str> {
        self.inner.vapid()
    }

    /// Reports whether the server accepts only UTF-8 messages.
    #[must_use]
    pub fn utf8only(&self) -> bool {
        self.inner.utf8only()
    }

    /// Reports whether extended `WHO` syntax is available.
    #[must_use]
    pub fn whox(&self) -> bool {
        self.inner.whox()
    }

    /// Reports whether authentication is required.
    #[must_use]
    pub fn account_required(&self) -> bool {
        self.inner.account_required()
    }

    /// Reports whether safe `LIST` behavior is available.
    #[must_use]
    pub fn safelist(&self) -> bool {
        self.inner.safelist()
    }

    /// Reports whether the server permits clients to send without local pacing.
    #[must_use]
    pub fn saferate(&self) -> bool {
        self.inner.saferate()
    }

    /// Returns the user mode used to identify bots.
    #[must_use]
    pub fn bot_mode(&self) -> Option<char> {
        self.inner.bot_mode()
    }

    /// Returns the monitor-list limit.
    #[must_use]
    pub fn monitor(&self) -> Option<LimitValue> {
        self.inner.monitor()
    }

    /// Returns the history request limit.
    #[must_use]
    pub fn chathistory(&self) -> Option<LimitValue> {
        self.inner.chathistory()
    }

    /// Returns supported message-reference types in preference order.
    #[must_use]
    pub fn msgreftypes(&self) -> Vec<&str> {
        self.inner.msgreftypes()
    }

    /// Returns the server's client-only tag filtering policy.
    #[must_use]
    pub fn clienttagdeny(&self) -> ClientTagDeny<'_> {
        self.inner.clienttagdeny()
    }

    /// Returns the ban-exception mode, using `e` for a bare token.
    #[must_use]
    pub fn excepts(&self) -> Option<char> {
        self.inner.excepts()
    }

    /// Returns the invite-exception mode, using `I` for a bare token.
    #[must_use]
    pub fn invex(&self) -> Option<char> {
        self.inner.invex()
    }

    /// Returns the extended-ban prefix and supported types.
    #[must_use]
    pub fn extban(&self) -> Option<ExtbanSyntax<'_>> {
        self.inner.extban()
    }

    /// Returns extended-ban names that refer to accounts.
    #[must_use]
    pub fn accountextban(&self) -> Vec<&str> {
        self.inner.accountextban()
    }

    /// Returns the case-insensitive `LIST` search extension letters.
    #[must_use]
    pub fn elist(&self) -> Option<&str> {
        self.inner.elist()
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use papo_proto::LineRef;

    #[test]
    fn exposes_read_only_protocol_views() {
        let support = super::Isupport::default();
        assert_eq!(support.get("network"), super::Advertised::Absent);
        assert_eq!(support.casemapping(), None);
        assert_eq!(support.chantypes(), "#&");
        assert_eq!(support.nicklen(), None);
        assert_eq!(support.msgreftypes(), ["msgid", "timestamp"]);
        assert!(!support.whox());
    }

    fn assert_limits_and_lengths(support: &super::Isupport) {
        assert!(support.targmax().iter().any(|limit| {
            limit.name == "MONITOR" && limit.limit == Some(super::LimitValue::Limited(100))
        }));
        assert_eq!(
            support.targmax_for("monitor"),
            Some(super::LimitValue::Limited(100))
        );
        assert_eq!(support.maxtargets(), Some(super::LimitValue::Limited(4)));
        assert_eq!(support.modes(), Some(4));
        assert_eq!(
            support.chanlimit(),
            [super::Limit {
                name: "#",
                limit: Some(super::LimitValue::Limited(100))
            }]
        );
        assert_eq!(
            support.maxlist(),
            [super::Limit {
                name: "beI",
                limit: Some(super::LimitValue::Limited(100))
            }]
        );
        assert_eq!(support.channellen(), Some(64));
        assert_eq!(support.topiclen(), Some(390));
        assert_eq!(support.kicklen(), Some(390));
        assert_eq!(support.awaylen(), Some(390));
        assert_eq!(support.namelen(), Some(150));
        assert_eq!(support.monitor(), Some(super::LimitValue::Limited(100)));
        assert_eq!(
            support.chathistory(),
            Some(super::LimitValue::Limited(1000))
        );
    }

    #[test]
    fn exposes_a_complete_server_advertisement() {
        let mut inner = papo_proto::Isupport::default();
        for line in [
            ":ergo.test 005 papo-fixture AWAYLEN=390 BOT=B CASEMAPPING=ascii CHANLIMIT=#:100 CHANMODES=Ibe,k,fl,CEMRUimnstu CHANNELLEN=64 CHANTYPES=# CHATHISTORY=1000 ELIST=U EXCEPTS EXTBAN=,m FORWARD=f INVEX :are supported by this server",
            ":ergo.test 005 papo-fixture KICKLEN=390 MAXLIST=beI:100 MAXTARGETS=4 MODES=4 MONITOR=100 MSGREFTYPES=msgid,timestamp NETWORK=ErgoTest NICKLEN=32 PREFIX=(qaohv)~&@%+ SAFELIST SAFERATE STATUSMSG=~&@%+ TARGMAX=NAMES:1,LIST:1,KICK:,WHOIS:1,USERHOST:10,PRIVMSG:4,TAGMSG:4,NOTICE:4,MONITOR:100 :are supported by this server",
            ":ergo.test 005 papo-fixture TOPICLEN=390 NAMELEN=150 UTF8ONLY WHOX draft/ACCOUNTREQUIRED draft/ICON=https://cdn.example.net/ergo-{size}.png VAPID=BN2Zr6VZlaXr1w :are supported by this server",
        ] {
            inner.apply(&LineRef::parse(line).unwrap());
        }
        let support = super::Isupport::from_proto(inner);

        assert_eq!(support.casemapping(), Some(crate::Casemapping::Ascii));

        assert_eq!(
            support.prefix(),
            [
                crate::Prefix::__new('q', '~'),
                crate::Prefix::__new('a', '&'),
                crate::Prefix::__new('o', '@'),
                crate::Prefix::__new('h', '%'),
                crate::Prefix::__new('v', '+'),
            ]
        );
        assert_eq!(support.chanmodes().unwrap().never, "CEMRUimnstu");
        assert_eq!(support.statusmsg(), Some("~&@%+"));
        assert!(support.is_channel("#papo"));
        assert!(!support.is_channel("&local"));
        assert_limits_and_lengths(&support);
        assert_eq!(support.network(), Some("ErgoTest"));
        assert_eq!(
            support.icon(),
            Some("https://cdn.example.net/ergo-{size}.png")
        );
        assert_eq!(
            support.icon_url(128).as_deref(),
            Some("https://cdn.example.net/ergo-128.png")
        );
        #[cfg(feature = "unstable")]
        assert_eq!(support.vapid(), Some("BN2Zr6VZlaXr1w"));
        assert!(support.utf8only());
        assert!(support.account_required());
        assert!(support.safelist());
        assert!(support.saferate());
        assert_eq!(support.bot_mode(), Some('B'));
        assert_eq!(support.excepts(), Some('e'));
        assert_eq!(support.invex(), Some('I'));
        assert_eq!(
            support.extban(),
            Some(super::ExtbanSyntax {
                prefix: None,
                types: "m"
            })
        );
        assert!(support.accountextban().is_empty());
        assert_eq!(support.elist(), Some("U"));

        let absent = super::Isupport::default();
        assert!(!absent.utf8only());
        assert!(!absent.account_required());
        assert!(!absent.safelist());
        assert!(!absent.saferate());
    }
}
