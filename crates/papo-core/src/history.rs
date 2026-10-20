use std::time::SystemTime;
use web_time::Instant;

use papo_proto::{LimitValue, Line, time};

use crate::{Feature, RequestError, RequestId, Session};

const LOCAL_HISTORY_LIMIT: usize = 1_000;

/// A message reference accepted by `CHATHISTORY`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MsgRef {
    /// A network-unique message identifier.
    Msgid(String),
    /// A UTC server timestamp.
    Timestamp(SystemTime),
    /// The open-ended `*` reference accepted by `LATEST`.
    Any,
}

/// The newest server read marker known for one target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadMarker {
    /// Channel or nickname to which the marker applies.
    pub target: String,
    /// Marker time, or `None` when the server reports `*`.
    pub time: Option<SystemTime>,
}

/// The newest live message remembered for one target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastSeen {
    /// Channel or nickname to which the message belongs.
    pub target: String,
    /// Network-unique message identifier.
    pub msgid: String,
    /// Server-assigned message time.
    pub time: SystemTime,
}

/// A history query supported by `CHATHISTORY`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HistoryQuery {
    /// The newest messages, optionally following a reference.
    Latest {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Reference after which messages are requested, or `None` for `*`.
        after: Option<MsgRef>,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages immediately before a reference.
    Before {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Exclusive upper bound.
        before: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages immediately after a reference.
    After {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Exclusive lower bound.
        after: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages surrounding a reference.
    Around {
        /// Channel or nickname whose history is requested.
        target: String,
        /// Reference around which messages are requested.
        around: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Messages between two exclusive references.
    Between {
        /// Channel or nickname whose history is requested.
        target: String,
        /// First exclusive bound.
        start: MsgRef,
        /// Second exclusive bound.
        end: MsgRef,
        /// Maximum number of returned messages.
        limit: usize,
    },
    /// Targets that received messages during a time range.
    Targets {
        /// Inclusive beginning of the search range.
        from: SystemTime,
        /// Inclusive end of the search range.
        to: SystemTime,
        /// Maximum number of returned targets.
        limit: usize,
    },
}

impl Session {
    /// Queues a labelled `CHATHISTORY` request using the server's advertised limits.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when history was not negotiated,
    /// [`RequestError::InvalidMessageReference`] when the server cannot accept a reference,
    /// or an error documented by [`Session::request`].
    pub fn chathistory(
        &mut self,
        now: Instant,
        query: HistoryQuery,
    ) -> Result<RequestId, RequestError> {
        let requested = query.limit();
        let advertised = match self.isupport.value().chathistory() {
            Some(LimitValue::Limited(limit)) => limit,
            Some(LimitValue::Unlimited) | None => LOCAL_HISTORY_LIMIT,
        };
        let supported = self.isupport.value().msgreftypes();
        let query = query.with_timestamp_fallback(&self.state, &supported)?;
        let params = query.into_params(
            requested.min(advertised).min(LOCAL_HISTORY_LIMIT),
            &supported,
        )?;
        self.request_for_feature(now, Line::new("CHATHISTORY", params), Feature::History)
    }

    /// Sets the read marker for `target` to a UTC timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when read markers were not negotiated,
    /// or an error documented by [`Session::request`].
    pub fn mark_read(
        &mut self,
        now: Instant,
        target: &str,
        time: SystemTime,
    ) -> Result<RequestId, RequestError> {
        self.request_for_feature(
            now,
            Line::new(
                "MARKREAD",
                [
                    target.to_owned(),
                    format!(
                        "timestamp={}",
                        time::format(time).ok_or(RequestError::InvalidMessageReference)?
                    ),
                ],
            ),
            Feature::ReadMarkers,
        )
    }

    /// Queries the current read marker for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when read markers were not negotiated,
    /// or an error documented by [`Session::request_with_matcher`].
    pub fn read_marker(&mut self, now: Instant, target: &str) -> Result<RequestId, RequestError> {
        self.require_feature(Feature::ReadMarkers)?;
        self.request_with_matcher(
            now,
            Line::new("MARKREAD", [target]),
            crate::ResponseMatcher::ReadMarker(target.to_owned()),
        )
    }
}

impl HistoryQuery {
    fn limit(&self) -> usize {
        match self {
            Self::Latest { limit, .. }
            | Self::Before { limit, .. }
            | Self::After { limit, .. }
            | Self::Around { limit, .. }
            | Self::Between { limit, .. }
            | Self::Targets { limit, .. } => *limit,
        }
    }

    fn into_params(self, limit: usize, supported: &[&str]) -> Result<Vec<String>, RequestError> {
        let encode = |reference| encode_reference(reference, supported);
        let bound = |reference| match reference {
            MsgRef::Any => Err(RequestError::InvalidMessageReference),
            reference => encode(reference),
        };
        let (command, mut params) = match self {
            Self::Latest { target, after, .. } => (
                "LATEST",
                vec![target, encode(after.unwrap_or(MsgRef::Any))?],
            ),
            Self::Before { target, before, .. } => ("BEFORE", vec![target, bound(before)?]),
            Self::After { target, after, .. } => ("AFTER", vec![target, bound(after)?]),
            Self::Around { target, around, .. } => ("AROUND", vec![target, bound(around)?]),
            Self::Between {
                target, start, end, ..
            } => ("BETWEEN", vec![target, bound(start)?, bound(end)?]),
            Self::Targets { from, to, .. } => (
                "TARGETS",
                vec![
                    format!(
                        "timestamp={}",
                        time::format(from).ok_or(RequestError::InvalidMessageReference)?
                    ),
                    format!(
                        "timestamp={}",
                        time::format(to).ok_or(RequestError::InvalidMessageReference)?
                    ),
                ],
            ),
        };
        params.insert(0, command.to_owned());
        params.push(limit.to_string());
        Ok(params)
    }

    fn with_timestamp_fallback(
        self,
        state: &crate::State,
        supported: &[&str],
    ) -> Result<Self, RequestError> {
        let replace =
            |target: &str, reference| fallback_reference(state, supported, target, reference);
        Ok(match self {
            Self::Latest {
                target,
                after,
                limit,
            } => Self::Latest {
                after: after
                    .map(|reference| replace(&target, reference))
                    .transpose()?,
                target,
                limit,
            },
            Self::Before {
                target,
                before,
                limit,
            } => Self::Before {
                before: replace(&target, before)?,
                target,
                limit,
            },
            Self::After {
                target,
                after,
                limit,
            } => Self::After {
                after: replace(&target, after)?,
                target,
                limit,
            },
            Self::Around {
                target,
                around,
                limit,
            } => Self::Around {
                around: replace(&target, around)?,
                target,
                limit,
            },
            Self::Between {
                target,
                start,
                end,
                limit,
            } => Self::Between {
                start: replace(&target, start)?,
                end: replace(&target, end)?,
                target,
                limit,
            },
            targets @ Self::Targets { .. } => targets,
        })
    }
}

fn fallback_reference(
    state: &crate::State,
    supported: &[&str],
    target: &str,
    reference: MsgRef,
) -> Result<MsgRef, RequestError> {
    let MsgRef::Msgid(msgid) = reference else {
        return Ok(reference);
    };
    if supported.contains(&"msgid") {
        return Ok(MsgRef::Msgid(msgid));
    }
    state
        .last_seen(target)
        .filter(|seen| supported.contains(&"timestamp") && seen.msgid == msgid)
        .map(|seen| MsgRef::Timestamp(seen.time))
        .ok_or(RequestError::InvalidMessageReference)
}

fn encode_reference(reference: MsgRef, supported: &[&str]) -> Result<String, RequestError> {
    match reference {
        MsgRef::Msgid(value) if supported.contains(&"msgid") => Ok(format!("msgid={value}")),
        MsgRef::Timestamp(value) if supported.contains(&"timestamp") => Ok(format!(
            "timestamp={}",
            time::format(value).ok_or(RequestError::InvalidMessageReference)?
        )),
        MsgRef::Any => Ok("*".to_owned()),
        MsgRef::Msgid(_) | MsgRef::Timestamp(_) => Err(RequestError::InvalidMessageReference),
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoryQuery, MsgRef};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn encodes_latest_timestamp_at_millisecond_precision() {
        let params = HistoryQuery::Latest {
            target: "#papo".into(),
            after: Some(MsgRef::Timestamp(
                UNIX_EPOCH + Duration::new(1_760_529_342, 123_999_999),
            )),
            limit: 50,
        }
        .into_params(50, &["timestamp"])
        .unwrap();
        assert_eq!(
            params,
            [
                "LATEST",
                "#papo",
                "timestamp=2025-10-15T11:55:42.123Z",
                "50"
            ]
        );
    }

    #[test]
    fn encodes_target_bounds_as_timestamp_references() {
        let params = HistoryQuery::Targets {
            from: UNIX_EPOCH + Duration::new(1_760_529_342, 123_999_999),
            to: UNIX_EPOCH + Duration::new(1_760_529_402, 987_999_999),
            limit: 50,
        }
        .into_params(50, &["timestamp"])
        .unwrap();
        assert_eq!(
            params,
            [
                "TARGETS",
                "timestamp=2025-10-15T11:55:42.123Z",
                "timestamp=2025-10-15T11:56:42.987Z",
                "50"
            ]
        );
    }
}
