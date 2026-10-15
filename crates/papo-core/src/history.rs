use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
        let params = query.into_params(
            requested.min(advertised).min(LOCAL_HISTORY_LIMIT),
            &self.isupport.value().msgreftypes(),
        )?;
        self.request(now, Message::new("CHATHISTORY", params))
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
