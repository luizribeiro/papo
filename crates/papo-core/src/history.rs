use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use papo_proto::{LimitValue, Message};

use crate::{RequestError, RequestId, Session};

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
        if !self
            .caps
            .enabled()
            .keys()
            .any(|name| matches!(name.as_str(), "draft/chathistory" | "chathistory"))
        {
            return Err(RequestError::Unsupported("chathistory"));
        }
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
        self.request(now, Message::new("CHATHISTORY", params))
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
        if !self.supports_read_marker() {
            return Err(RequestError::Unsupported("read-marker"));
        }
        self.request(
            now,
            Message::new(
                "MARKREAD",
                [
                    target.to_owned(),
                    format!("timestamp={}", format_timestamp(time)?),
                ],
            ),
        )
    }

    /// Queries the current read marker for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::Unsupported`] when read markers were not negotiated,
    /// or [`RequestError::Build`] when the target cannot be encoded.
    pub fn read_marker(&mut self, target: &str) -> Result<(), RequestError> {
        if !self.supports_read_marker() {
            return Err(RequestError::Unsupported("read-marker"));
        }
        let line = Message::new("MARKREAD", [target]).to_line()?;
        self.outbox.transmit_raw(line.into());
        Ok(())
    }

    fn supports_read_marker(&self) -> bool {
        self.caps
            .enabled()
            .keys()
            .any(|name| matches!(name.as_str(), "draft/read-marker" | "read-marker"))
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
        let (command, mut params) = match self {
            Self::Latest { target, after, .. } => (
                "LATEST",
                vec![target, encode(after.unwrap_or(MsgRef::Any))?],
            ),
            Self::Before { target, before, .. } => ("BEFORE", vec![target, encode(before)?]),
            Self::After { target, after, .. } => ("AFTER", vec![target, encode(after)?]),
            Self::Around { target, around, .. } => ("AROUND", vec![target, encode(around)?]),
            Self::Between {
                target, start, end, ..
            } => ("BETWEEN", vec![target, encode(start)?, encode(end)?]),
            Self::Targets { from, to, .. } => (
                "TARGETS",
                vec![format_timestamp(from)?, format_timestamp(to)?],
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
        MsgRef::Timestamp(value) if supported.contains(&"timestamp") => {
            Ok(format!("timestamp={}", format_timestamp(value)?))
        }
        MsgRef::Any => Ok("*".to_owned()),
        MsgRef::Msgid(_) | MsgRef::Timestamp(_) => Err(RequestError::InvalidMessageReference),
    }
}

fn format_timestamp(time: SystemTime) -> Result<String, RequestError> {
    let millis = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_millis()),
        Err(error) => i128::try_from(error.duration().as_millis()).map(|value| -value),
    }
    .map_err(|_| RequestError::InvalidMessageReference)?;
    let seconds = millis.div_euclid(1_000);
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100 + year_of_era / 400);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    year += i128::from(month <= 2);
    if !(0..=9_999).contains(&year) {
        return Err(RequestError::InvalidMessageReference);
    }
    let second_of_day = seconds.rem_euclid(86_400);
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60,
        millis.rem_euclid(1_000)
    ))
}

pub(crate) fn parse_timestamp(value: &str) -> Option<SystemTime> {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || !matches!(
            (
                bytes[4], bytes[7], bytes[10], bytes[13], bytes[16], bytes[19], bytes[23]
            ),
            (b'-', b'-', b'T', b':', b':', b'.', b'Z')
        )
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| {
        bytes[range].iter().try_fold(0_i64, |value, byte| {
            byte.is_ascii_digit()
                .then_some(value * 10 + i64::from(byte - b'0'))
        })
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let millis = number(20..23)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        2 => 28 + i64::from(leap),
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if !(1..=month_days).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let days = era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + day_of_year
        - 719_468;
    let total_millis = (days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000 + millis;
    let duration = Duration::from_millis(total_millis.unsigned_abs());
    if total_millis >= 0 {
        UNIX_EPOCH.checked_add(duration)
    } else {
        UNIX_EPOCH.checked_sub(duration)
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
}
