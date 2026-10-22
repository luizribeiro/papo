use web_time::Instant;

use papo_proto::{Line, MetadataKey};

use crate::{Capabilities, Feature, RequestError, RequestId, Session, cap};

/// One server-returned metadata value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MetadataValue {
    /// Server-returned value.
    pub value: String,
    /// Visibility policy reported with the value.
    pub visibility: Visibility,
}

/// Visibility of a metadata value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Visibility {
    /// The value is public.
    Public,
    /// The server restricts the value according to the named policy.
    Restricted(String),
}

impl Visibility {
    pub(crate) fn from_wire(value: &str) -> Self {
        if value == "*" {
            Self::Public
        } else {
            Self::Restricted(value.to_owned())
        }
    }
}

/// A `METADATA` operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MetadataCommand {
    /// Gets selected keys for a user or channel.
    #[non_exhaustive]
    Get {
        /// Nickname or channel to query.
        target: String,
        /// Keys to query.
        keys: Vec<String>,
    },
    /// Lists all visible keys for a user or channel.
    #[non_exhaustive]
    List {
        /// Nickname or channel to query.
        target: String,
    },
    /// Sets or removes one key for a user or channel.
    #[non_exhaustive]
    Set {
        /// Nickname or channel to change.
        target: String,
        /// Key to change.
        key: String,
        /// New nonempty value, or `None` to remove the key.
        value: Option<String>,
    },
    /// Removes every metadata key from a user or channel.
    #[non_exhaustive]
    Clear {
        /// Nickname or channel to clear.
        target: String,
    },
    /// Subscribes to metadata keys.
    #[non_exhaustive]
    Sub {
        /// Keys to add to the subscription list.
        keys: Vec<String>,
    },
    /// Unsubscribes from metadata keys.
    #[non_exhaustive]
    Unsub {
        /// Keys to remove from the subscription list.
        keys: Vec<String>,
    },
    /// Lists current metadata subscriptions.
    Subs,
    /// Synchronizes metadata for a user or channel.
    #[non_exhaustive]
    Sync {
        /// Nickname or channel to synchronize.
        target: String,
    },
}

impl MetadataCommand {
    #[doc(hidden)]
    #[must_use]
    pub fn __get(target: String, keys: Vec<String>) -> Self {
        Self::Get { target, keys }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __list(target: String) -> Self {
        Self::List { target }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __set(target: String, key: String, value: Option<String>) -> Self {
        Self::Set { target, key, value }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __clear(target: String) -> Self {
        Self::Clear { target }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __sub(keys: Vec<String>) -> Self {
        Self::Sub { keys }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __unsub(keys: Vec<String>) -> Self {
        Self::Unsub { keys }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __sync(target: String) -> Self {
        Self::Sync { target }
    }
}

/// Server-advertised limits for `draft/metadata-2` or `draft/metadata-3`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetadataLimits {
    /// Whether subscriptions may be sent before connection registration completes.
    pub before_connect: bool,
    /// Maximum number of subscribed keys, when advertised.
    pub max_subscriptions: Option<usize>,
    /// Maximum keys stored for one user or channel, when advertised.
    pub max_keys: Option<usize>,
    /// Maximum metadata value length in bytes, when advertised.
    pub max_value_bytes: Option<usize>,
}

impl MetadataLimits {
    pub(crate) fn from_caps(caps: &Capabilities) -> Option<Self> {
        let value = caps.get(&cap::METADATA)?;
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
    /// Returns [`RequestError::EmptyText`] for an empty value,
    /// [`RequestError::Unsupported`] when metadata was not negotiated, or an error documented by
    /// [`Session::request`].
    pub fn metadata(
        &mut self,
        now: Instant,
        command: MetadataCommand,
    ) -> Result<RequestId, RequestError> {
        if matches!(
            &command,
            MetadataCommand::Set {
                value: Some(value),
                ..
            } if value.is_empty()
        ) {
            return Err(RequestError::EmptyText);
        }
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
        self.request_for_feature(
            now,
            Line::new("METADATA").with_params(params),
            Feature::Metadata,
        )
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

fn validate_subscriptions<'a>(
    keys: &[String],
    limits: MetadataLimits,
    current: impl Iterator<Item = &'a MetadataKey>,
) -> Result<(), RequestError> {
    validate_keys(keys)?;
    let actual = current
        .map(ToString::to_string)
        .chain(keys.iter().cloned())
        .collect::<std::collections::HashSet<_>>()
        .len();
    if let Some(maximum) = limits.max_subscriptions
        && actual > maximum
    {
        return Err(RequestError::TooManyMetadataSubscriptions { maximum, actual });
    }
    Ok(())
}
