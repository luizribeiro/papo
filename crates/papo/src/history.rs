use papo_core::HistoryQuery;

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

impl History<'_> {
    /// Requests the newest `limit` messages.
    ///
    /// # Errors
    ///
    /// Returns an error when history is unsupported or the request cannot complete.
    pub async fn latest(self, limit: usize) -> Result<HistoryPage, Error> {
        self.client
            .history_page(HistoryQuery::Latest {
                target: self.target,
                after: None,
                limit,
            })
            .await
    }
}
