//! Read-only views of server features advertised through `RPL_ISUPPORT`.

#[doc(inline)]
pub use papo_proto::{Advertised, ChanModes, ClientTagDeny, Extban, Limit, LimitValue, Prefix};

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

    /// Returns the server's name casemapping.
    #[must_use]
    pub fn casemapping(&self) -> Casemapping {
        self.inner.casemapping()
    }

    /// Returns the channel-type prefix characters.
    #[must_use]
    pub fn chantypes(&self) -> &str {
        self.inner.chantypes()
    }

    /// Returns channel membership prefixes from highest to lowest rank.
    #[must_use]
    pub fn prefix(&self) -> Vec<Prefix> {
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

    /// Reports whether a target starts with an advertised channel-type prefix.
    #[must_use]
    pub fn is_channel(&self, name: &str) -> bool {
        self.inner.is_channel(name)
    }

    /// Returns per-command target limits, defaulting to unlimited `JOIN` and `PART`.
    #[must_use]
    pub fn targmax(&self) -> Vec<Limit<'_>> {
        self.inner.targmax()
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
    pub fn extban(&self) -> Option<Extban<'_>> {
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

    /// Returns the HTTPS endpoint advertised for file uploads.
    ///
    /// Uploads use an HTTP `POST` whose response supplies the uploaded URL in its `Location`
    /// header. The caller authenticates that request with its own credentials; papo does not
    /// expose authentication secrets. Non-HTTPS endpoints are ignored.
    #[must_use]
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub fn filehost(&self) -> Option<&str> {
        ["draft/FILEHOST", "soju.im/FILEHOST"]
            .into_iter()
            .find_map(|name| match self.get(name) {
                Advertised::Value(url) if url.starts_with("https://") => Some(url),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn exposes_read_only_protocol_views() {
        let support = super::Isupport::default();
        assert_eq!(support.get("network"), super::Advertised::Absent);
        assert_eq!(support.chantypes(), "#&");
        assert_eq!(support.nicklen(), None);
        assert_eq!(support.msgreftypes(), ["msgid", "timestamp"]);
        assert!(!support.whox());
    }
}
