/// A network message identifier together with its conversation target.
///
/// Targets use exact string equality for [`Eq`]. When comparing identifiers
/// across server input, compare [`Self::target`] with the network's casemapping.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TargetedMessageId {
    target: String,
    msgid: String,
}

impl TargetedMessageId {
    /// Creates an identifier for `msgid` in `target`.
    #[must_use]
    pub fn new(target: impl Into<String>, msgid: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            msgid: msgid.into(),
        }
    }

    /// Returns the channel or nickname containing the message.
    ///
    /// Compare this value with other IRC targets using the network's casemapping.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the server-assigned message identifier.
    #[must_use]
    pub fn msgid(&self) -> &str {
        &self.msgid
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::TargetedMessageId;

    #[test]
    fn owns_both_parts_and_hashes_by_exact_value() {
        let id = TargetedMessageId::new("#Papo", "RfcBTzyxY120004");
        let mut ids = HashSet::from([id.clone()]);

        assert_eq!(id.target(), "#Papo");
        assert_eq!(id.msgid(), "RfcBTzyxY120004");
        assert!(ids.remove(&TargetedMessageId::new("#Papo", "RfcBTzyxY120004")));
        assert_ne!(id, TargetedMessageId::new("#papo", "RfcBTzyxY120004"));
    }
}
