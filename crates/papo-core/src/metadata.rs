use std::time::Instant;

use papo_proto::Message;

use crate::{Capabilities, Feature, RequestError, RequestId, Session};

/// One server-returned metadata value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataValue {
    /// Server-returned value.
    pub value: String,
    /// Visibility policy reported with the value.
    pub visibility: String,
}

/// A typed `FAIL METADATA` code.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MetadataError {
    /// The target is not valid for metadata operations.
    InvalidTarget,
    /// The key does not follow the metadata key syntax.
    KeyInvalid,
    /// The client cannot modify the key.
    KeyNoPermission,
    /// The requested key is not set.
    KeyNotSet,
    /// A server metadata limit was reached.
    LimitReached,
    /// Metadata changes are temporarily rate limited.
    RateLimited {
        /// Target named by the failure.
        target: String,
        /// Key named by the failure.
        key: String,
        /// Delay before retrying, or `None` when the server sent `*`.
        retry_after: Option<std::time::Duration>,
    },
    /// The subscription limit interrupted ordered processing.
    TooManySubscriptions {
        /// First key the server did not process.
        first_unprocessed_key: String,
    },
    /// The metadata subcommand is not recognized.
    SubcommandInvalid,
    /// The supplied metadata value is invalid.
    ValueInvalid,
}

impl MetadataError {
    pub(crate) fn parse(command: &str, code: &str, context: &[String]) -> Option<Self> {
        if !command.eq_ignore_ascii_case("METADATA") {
            return None;
        }
        match code.to_ascii_uppercase().as_str() {
            "INVALID_TARGET" => Some(Self::InvalidTarget),
            "KEY_INVALID" => Some(Self::KeyInvalid),
            "KEY_NO_PERMISSION" => Some(Self::KeyNoPermission),
            "KEY_NOT_SET" => Some(Self::KeyNotSet),
            "LIMIT_REACHED" => Some(Self::LimitReached),
            "RATE_LIMITED" => {
                let retry_after = match context.get(2)?.as_str() {
                    "*" => None,
                    seconds => Some(std::time::Duration::from_secs(seconds.parse().ok()?)),
                };
                Some(Self::RateLimited {
                    target: context.first()?.clone(),
                    key: context.get(1)?.clone(),
                    retry_after,
                })
            }
            "TOO_MANY_SUBS" => Some(Self::TooManySubscriptions {
                first_unprocessed_key: context.first()?.clone(),
            }),
            "SUBCOMMAND_INVALID" => Some(Self::SubcommandInvalid),
            "VALUE_INVALID" => Some(Self::ValueInvalid),
            _ => None,
        }
    }
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
    (!keys.is_empty()
        && keys.iter().all(|key| {
            !key.is_empty()
                && key.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_./-".contains(&byte)
                })
        }))
    .then_some(())
    .ok_or(RequestError::InvalidMetadataKey)
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
