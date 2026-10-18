use web_time::Instant;

use papo_proto::{Message, MetadataKey};

use crate::{Capabilities, Feature, RequestError, RequestId, Session};

/// One server-returned metadata value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataValue {
    /// Server-returned value.
    pub value: String,
    /// Visibility policy reported with the value.
    pub visibility: String,
}

/// A `METADATA` operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MetadataCommand {
    /// Gets selected keys for a target.
    Get {
        /// Nickname or channel to query.
        target: String,
        /// Keys to query.
        keys: Vec<String>,
    },
    /// Lists all visible keys for a target.
    List {
        /// Nickname or channel to query.
        target: String,
    },
    /// Sets or removes one key for a target.
    Set {
        /// Nickname or channel to change.
        target: String,
        /// Key to change.
        key: String,
        /// New value, or `None` to remove the key.
        value: Option<String>,
    },
    /// Removes every metadata key from a target.
    Clear {
        /// Nickname or channel to clear.
        target: String,
    },
    /// Subscribes to metadata keys.
    Sub {
        /// Keys to add to the subscription list.
        keys: Vec<String>,
    },
    /// Unsubscribes from metadata keys.
    Unsub {
        /// Keys to remove from the subscription list.
        keys: Vec<String>,
    },
    /// Lists current metadata subscriptions.
    Subs,
    /// Synchronizes metadata for a target.
    Sync {
        /// Nickname or channel to synchronize.
        target: String,
    },
}

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

impl Session {
    /// Queues a labelled metadata request.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when metadata was not negotiated, or an error
    /// documented by [`Session::request`].
    pub fn metadata(
        &mut self,
        now: Instant,
        command: MetadataCommand,
    ) -> Result<RequestId, RequestError> {
        self.require_feature(Feature::Metadata)?;
        let Some(limits) = self.metadata_limits() else {
            return Err(RequestError::unsupported(Feature::Metadata));
        };
        let mut params = Vec::new();
        match command {
            MetadataCommand::Get { target, keys } => {
                validate_keys(&keys)?;
                params.extend([target, "GET".into()]);
                params.extend(keys);
            }
            MetadataCommand::List { target } => params.extend([target, "LIST".into()]),
            MetadataCommand::Set { target, key, value } => {
                validate_keys(std::slice::from_ref(&key))?;
                if let (Some(maximum), Some(value)) = (limits.max_value_bytes, value.as_ref())
                    && value.len() > maximum
                {
                    return Err(RequestError::MetadataValueTooLong {
                        maximum,
                        actual: value.len(),
                    });
                }
                params.extend([target, "SET".into(), key]);
                params.extend(value);
            }
            MetadataCommand::Clear { target } => params.extend([target, "CLEAR".into()]),
            MetadataCommand::Sub { keys } => {
                validate_subscriptions(&keys, limits, self.state.metadata_subscriptions())?;
                params.extend(["*".into(), "SUB".into()]);
                params.extend(keys);
            }
            MetadataCommand::Unsub { keys } => {
                validate_keys(&keys)?;
                params.extend(["*".into(), "UNSUB".into()]);
                params.extend(keys);
            }
            MetadataCommand::Subs => params.extend(["*".into(), "SUBS".into()]),
            MetadataCommand::Sync { target } => params.extend([target, "SYNC".into()]),
        }
        self.request_for_feature(now, Message::new("METADATA", params), Feature::Metadata)
    }
}

fn validate_keys(keys: &[String]) -> Result<(), RequestError> {
    if keys.is_empty() {
        return Err(RequestError::InvalidMetadataKey {
            key: String::new(),
            reason: papo_proto::InvalidIdentifier::Empty,
        });
    }
    for key in keys {
        if let Err(reason) = MetadataKey::from(key.as_str()).check() {
            return Err(RequestError::InvalidMetadataKey {
                key: key.clone(),
                reason,
            });
        }
    }
    Ok(())
}

fn validate_subscriptions(
    keys: &[String],
    limits: MetadataLimits,
    current: &[String],
) -> Result<(), RequestError> {
    validate_keys(keys)?;
    let actual = current
        .iter()
        .map(String::as_str)
        .chain(keys.iter().map(String::as_str))
        .collect::<std::collections::HashSet<_>>()
        .len();
    if let Some(maximum) = limits.max_subscriptions
        && actual > maximum
    {
        return Err(RequestError::TooManyMetadataSubscriptions { maximum, actual });
    }
    Ok(())
}
