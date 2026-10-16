use std::{future::Future, pin::Pin, time::SystemTime};

use papo_core::{HistoryQuery, MsgRef};

use crate::{Client, Error, MessageEvent};

/// A page of requested chat history.
#[derive(Clone, Debug)]
pub struct HistoryPage {
    /// Messages in server delivery order.
    pub messages: Vec<MessageEvent>,
    /// Whether the server reported that no further page exists.
    pub end: bool,
}

/// Builds a history request for one channel or nickname.
pub struct History<'a> {
    pub(crate) client: &'a Client,
    pub(crate) target: String,
}

impl<'a> History<'a> {
    /// Requests the newest `limit` messages.
    #[must_use]
    pub fn latest(self, limit: usize) -> HistoryRequest<'a> {
        HistoryRequest {
            client: self.client,
            query: Ok(HistoryQuery::Latest {
                target: self.target,
                after: None,
                limit,
            }),
        }
    }

    /// Requests messages before `reference`.
    #[must_use]
    pub fn before(self, reference: impl Into<HistoryReference>) -> HistoryRequest<'a> {
        let query = reference.into().0.ok_or(Error::Unsupported("message id"));
        HistoryRequest {
            client: self.client,
            query: query.map(|before| HistoryQuery::Before {
                target: self.target,
                before,
                limit: 100,
            }),
        }
    }

    /// Requests messages after `reference`.
    #[must_use]
    pub fn after(self, reference: impl Into<HistoryReference>) -> HistoryRequest<'a> {
        let query = reference.into().0.ok_or(Error::Unsupported("message id"));
        HistoryRequest {
            client: self.client,
            query: query.map(|after| HistoryQuery::After {
                target: self.target,
                after,
                limit: 100,
            }),
        }
    }

    /// Requests messages surrounding `reference`.
    #[must_use]
    pub fn around(self, reference: impl Into<HistoryReference>) -> HistoryRequest<'a> {
        let query = reference.into().0.ok_or(Error::Unsupported("message id"));
        HistoryRequest {
            client: self.client,
            query: query.map(|around| HistoryQuery::Around {
                target: self.target,
                around,
                limit: 100,
            }),
        }
    }

    /// Requests messages between two exclusive references.
    #[must_use]
    pub fn between(
        self,
        start: impl Into<HistoryReference>,
        end: impl Into<HistoryReference>,
    ) -> HistoryRequest<'a> {
        let start = start.into().0.ok_or(Error::Unsupported("message id"));
        let query = start.and_then(|start| {
            end.into()
                .0
                .ok_or(Error::Unsupported("message id"))
                .map(|end| HistoryQuery::Between {
                    target: self.target,
                    start,
                    end,
                    limit: 100,
                })
        });
        HistoryRequest {
            client: self.client,
            query,
        }
    }
}

/// A message identifier or timestamp accepted by a history query.
pub struct HistoryReference(Option<MsgRef>);

impl From<&str> for HistoryReference {
    fn from(value: &str) -> Self {
        Self(Some(MsgRef::Msgid(value.to_owned())))
    }
}

impl From<&String> for HistoryReference {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<String> for HistoryReference {
    fn from(value: String) -> Self {
        Self(Some(MsgRef::Msgid(value)))
    }
}

impl From<SystemTime> for HistoryReference {
    fn from(value: SystemTime) -> Self {
        Self(Some(MsgRef::Timestamp(value)))
    }
}

impl From<&MessageEvent> for HistoryReference {
    fn from(value: &MessageEvent) -> Self {
        Self(value.msgid().map(|msgid| MsgRef::Msgid(msgid.to_owned())))
    }
}

/// A history request with a configurable page limit.
pub struct HistoryRequest<'a> {
    client: &'a Client,
    query: Result<HistoryQuery, Error>,
}

impl HistoryRequest<'_> {
    /// Sets the maximum number of returned messages.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        if let Ok(
            HistoryQuery::Latest { limit: value, .. }
            | HistoryQuery::Before { limit: value, .. }
            | HistoryQuery::After { limit: value, .. }
            | HistoryQuery::Around { limit: value, .. }
            | HistoryQuery::Between { limit: value, .. },
        ) = &mut self.query
        {
            *value = limit;
        }
        self
    }
}

impl<'a> IntoFuture for HistoryRequest<'a> {
    type Output = Result<HistoryPage, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        match self.query {
            Ok(query) => Box::pin(self.client.history_page(query)),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use papo_core::{HistoryQuery, MsgRef};
    use papo_proto::Message;

    use super::HistoryReference;

    #[test]
    fn converts_ids_timestamps_and_message_events() {
        assert!(
            matches!(HistoryReference::from("2Zl7JeYG6I8fG9q1lJ55BQ").0, Some(MsgRef::Msgid(id)) if id == "2Zl7JeYG6I8fG9q1lJ55BQ")
        );
        assert!(
            matches!(HistoryReference::from(UNIX_EPOCH).0, Some(MsgRef::Timestamp(time)) if time == UNIX_EPOCH)
        );
        let event = crate::MessageEvent::from_history(
            Message::privmsg("#papo", "earlier").with_tag("msgid", "RfcBTzyxY120004"),
            crate::Client::disconnected_for_test(),
            UNIX_EPOCH,
        )
        .unwrap();
        assert!(
            matches!(HistoryReference::from(&event).0, Some(MsgRef::Msgid(id)) if id == "RfcBTzyxY120004")
        );
    }

    #[test]
    fn builds_after_around_and_between_queries_with_limits() {
        let client = crate::Client::disconnected_for_test();
        let after = client
            .history("#papo")
            .after(UNIX_EPOCH + Duration::from_secs(42))
            .limit(51);
        assert!(matches!(
            after.query,
            Ok(HistoryQuery::After { limit: 51, .. })
        ));
        let around = client.history("#papo").around("RfcBTzyxY120000").limit(52);
        assert!(matches!(
            around.query,
            Ok(HistoryQuery::Around { limit: 52, .. })
        ));
        let between = client
            .history("#papo")
            .between("RfcBTzyxY119998", "RfcBTzyxY120003")
            .limit(53);
        assert!(matches!(
            between.query,
            Ok(HistoryQuery::Between { limit: 53, .. })
        ));
    }
}
