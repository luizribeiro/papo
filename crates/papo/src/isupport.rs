//! Read-only views of server features advertised through `RPL_ISUPPORT`.

pub use papo_proto::{Advertised, ChanModes, ClientTagDeny, Extban, Limit, LimitValue, Prefix};

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
}

#[cfg(test)]
mod tests {
    #[test]
    fn exposes_read_only_protocol_views() {
        let support = super::Isupport::default();
        assert_eq!(support.get("network"), super::Advertised::Absent);
        assert_eq!(support.chantypes(), "#&");
        assert_eq!(support.nicklen(), None);
    }
}
