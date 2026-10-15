use crate::Capabilities;

/// Server-advertised limits for `draft/metadata-2` or `draft/metadata-3`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetadataLimits {
    /// Whether subscriptions may be sent before connection registration completes.
    pub before_connect: bool,
    /// Maximum number of subscribed keys, when advertised.
    pub max_subscriptions: Option<usize>,
    /// Maximum keys stored for one target, when advertised.
    pub max_keys: Option<usize>,
    /// Maximum metadata value length in bytes, when advertised.
    pub max_value_bytes: Option<usize>,
}

impl MetadataLimits {
    pub(crate) fn from_caps(caps: &Capabilities) -> Option<Self> {
        let value = caps
            .get("draft/metadata-3")
            .or_else(|| caps.get("draft/metadata-2"))?;
        Some(Self::parse(value.as_deref()))
    }

    fn parse(value: Option<&str>) -> Self {
        let mut limits = Self::default();
        for token in value.unwrap_or("").split(',') {
            let (key, value) = token.split_once('=').map_or((token, None), |(key, value)| {
                (key, value.parse::<usize>().ok())
            });
            match key {
                "before-connect" => limits.before_connect = true,
                "max-subs" => limits.max_subscriptions = value,
                "max-keys" => limits.max_keys = value,
                "max-value-bytes" => limits.max_value_bytes = value,
                _ => {}
            }
        }
        limits
    }
}
