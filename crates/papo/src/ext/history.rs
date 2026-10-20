use std::{future::IntoFuture, time::SystemTime};

use papo_core::{HistoryQuery, MsgRef, Response};
use papo_proto::{ModeChanges, Reaction};
use tokio::sync::oneshot;

use crate::{
    Client, Error, Message as ChatMessage, Origin, RawLine, Source,
    driver::Command,
    event::{
        Joined, Kicked, ModeChanged, NickChanged, Parted, Quit, Reacted, Redacted, TopicChanged,
    },
};

/// One typed entry in a history page.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum HistoryItem {
    /// A replayed `PRIVMSG` or `NOTICE`.
    Message(ChatMessage),
    /// A replayed reaction or unreaction.
    Reacted(Reacted),
    /// A replayed message redaction.
    Redacted(Redacted),
    /// A replayed channel join.
    Joined(Joined),
    /// A replayed channel part.
    Parted(Parted),
    /// A replayed user quit.
    Quit(Quit),
    /// A replayed channel kick.
    Kicked(Kicked),
    /// A replayed nickname change.
    NickChanged(NickChanged),
    /// A replayed topic change.
    TopicChanged(TopicChanged),
    /// A replayed mode change.
    ModeChanged(ModeChanged),
    /// A replayed line without a more specific typed representation.
    ///
    /// A later minor release may refine a command currently classified here.
    Other(RawLine),
}

impl HistoryItem {
    /// Returns the message when this item is [`HistoryItem::Message`].
    #[must_use]
    pub const fn message(&self) -> Option<&ChatMessage> {
        match self {
            Self::Message(message) => Some(message),
            _ => None,
        }
    }
}

pub(crate) fn history_item(
    item: papo_core::HistoryItem,
    client: &Client,
    received_at: SystemTime,
    origin: Origin,
) -> HistoryItem {
    match item {
        papo_core::HistoryItem::Message(item) => {
            let message = match origin {
                Origin::History => ChatMessage::from_history(item.message, client, received_at),
                _ => ChatMessage::from_replay(item.message, client, received_at),
            };
            message.map_or_else(
                |raw| HistoryItem::Other(RawLine::from_proto(&raw)),
                HistoryItem::Message,
            )
        }
        papo_core::HistoryItem::Reacted(item) => typed_or_raw(&item.message, reaction),
        papo_core::HistoryItem::Redacted(item) => typed_or_raw(&item.message, redaction),
        papo_core::HistoryItem::Joined(item) => {
            typed_or_raw(&item.message, |message| joined(message, client))
        }
        papo_core::HistoryItem::Parted(item) => {
            typed_or_raw(&item.message, |message| parted(message, client))
        }
        papo_core::HistoryItem::Quit(item) => typed_or_raw(&item.message, quit),
        papo_core::HistoryItem::Kicked(item) => {
            typed_or_raw(&item.message, |message| kicked(message, client))
        }
        papo_core::HistoryItem::NickChanged(item) => {
            typed_or_raw(&item.message, |message| nick_changed(message, client))
        }
        papo_core::HistoryItem::TopicChanged(item) => typed_or_raw(&item.message, topic_changed),
        papo_core::HistoryItem::ModeChanged(item) => {
            typed_or_raw(&item.message, |message| mode_changed(message, client))
        }
        item => {
            let raw = item.into_raw().message;
            HistoryItem::Other(RawLine::from_proto(&raw))
        }
    }
}

fn typed_or_raw<F>(raw: &papo_proto::Message, convert: F) -> HistoryItem
where
    F: FnOnce(&papo_proto::LineRef<'_>) -> Option<HistoryItem>,
{
    convert(&raw.as_ref()).unwrap_or_else(|| HistoryItem::Other(RawLine::from_proto(raw)))
}

fn reaction(message: &papo_proto::LineRef<'_>) -> Option<HistoryItem> {
    let reaction = Reaction::parse(message)?;
    let target = message.param(0)?.to_owned();
    let reply_to = reaction.reply_to.clone();
    let (reaction, removed) = reaction
        .react
        .map(|value| (value, false))
        .or_else(|| reaction.unreact.map(|value| (value, true)))?;
    Some(HistoryItem::Reacted(Reacted {
        source: message.source().map(Source::from_proto),
        target: target.into(),
        message: reply_to.into(),
        reaction,
        removed,
    }))
}

fn redaction(message: &papo_proto::LineRef<'_>) -> Option<HistoryItem> {
    Some(HistoryItem::Redacted(Redacted {
        source: message.source().map(Source::from_proto),
        target: message.param(0)?.into(),
        message: message.param(1)?.into(),
        reason: message
            .param(2)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned),
    }))
}

fn joined(message: &papo_proto::LineRef<'_>, client: &Client) -> Option<HistoryItem> {
    let nick = source_nick(message)?;
    Some(HistoryItem::Joined(Joined {
        channel: message.param(0)?.into(),
        is_self: is_self(client, &nick),
        nick: nick.into(),
    }))
}

fn parted(message: &papo_proto::LineRef<'_>, client: &Client) -> Option<HistoryItem> {
    let nick = source_nick(message)?;
    Some(HistoryItem::Parted(Parted {
        channel: message.param(0)?.into(),
        is_self: is_self(client, &nick),
        nick: nick.into(),
        reason: message.param(1).map(str::to_owned),
    }))
}

fn quit(message: &papo_proto::LineRef<'_>) -> Option<HistoryItem> {
    Some(HistoryItem::Quit(Quit {
        nick: source_nick(message)?.into(),
        reason: message.param(0).map(str::to_owned),
        channels: Vec::new(),
    }))
}

fn kicked(message: &papo_proto::LineRef<'_>, client: &Client) -> Option<HistoryItem> {
    let nick = message.param(1)?.to_owned();
    Some(HistoryItem::Kicked(Kicked {
        channel: message.param(0)?.into(),
        is_self: is_self(client, &nick),
        nick: nick.into(),
        by: Source::from_proto(message.source()?),
        reason: message.param(2).map(str::to_owned),
    }))
}

fn nick_changed(message: &papo_proto::LineRef<'_>, client: &Client) -> Option<HistoryItem> {
    let old = source_nick(message)?;
    Some(HistoryItem::NickChanged(NickChanged {
        is_self: is_self(client, &old),
        old: old.into(),
        new: message.param(0)?.into(),
    }))
}

fn topic_changed(message: &papo_proto::LineRef<'_>) -> Option<HistoryItem> {
    Some(HistoryItem::TopicChanged(TopicChanged {
        channel: message.param(0)?.into(),
        topic: message
            .param(1)
            .filter(|topic| !topic.is_empty())
            .map(str::to_owned),
        by: Source::from_proto(message.source()?),
    }))
}

fn mode_changed(message: &papo_proto::LineRef<'_>, client: &Client) -> Option<HistoryItem> {
    let state = client.state();
    let changes = ModeChanges::parse(
        message.param(1)?,
        message.params().skip(2),
        state.isupport().inner(),
    )
    .map(|change| crate::event::ModeChange {
        set: change.set,
        mode: change.mode,
        kind: crate::ModeKind::from_proto(change.kind),
        arg: change.arg.map(str::to_owned),
    })
    .collect();
    Some(HistoryItem::ModeChanged(ModeChanged {
        target: message.param(0)?.into(),
        by: Source::from_proto(message.source()?),
        changes,
    }))
}

fn source_nick(message: &papo_proto::LineRef<'_>) -> Option<String> {
    match message.source()? {
        papo_proto::Source::User { nick, .. } => Some(nick.to_owned()),
        papo_proto::Source::Server(_) => None,
    }
}

fn is_self(client: &Client, nick: &str) -> bool {
    let state = client.state();
    state.isupport().casemapping().eq(state.nick(), nick)
}

/// A page of requested chat history.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HistoryPage {
    /// Target whose history was requested.
    pub target: crate::Target,
    /// Replayed entries in server delivery order.
    pub items: Vec<HistoryItem>,
    /// Whether the server reported that no further page exists.
    pub end: bool,
}

/// A target with the timestamp of its newest available history entry.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HistoryTarget {
    /// Target with available history.
    pub target: crate::Target,
    /// Timestamp of the newest available entry.
    pub latest: SystemTime,
}

impl HistoryPage {
    /// Iterates over chat messages, excluding replayed activity items.
    pub fn messages(&self) -> impl Iterator<Item = &ChatMessage> {
        self.items.iter().filter_map(HistoryItem::message)
    }
}

/// Builds a history request for one channel or nickname.
pub struct History {
    pub(crate) client: Client,
    pub(crate) target: String,
}

impl History {
    /// Requests the newest `limit` messages.
    #[must_use]
    pub fn latest(self, limit: usize) -> HistoryRequest {
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
    pub fn before(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        let query = reference.into().0.ok_or(Error::InvalidInput(
            crate::InvalidInput::UnusableMessageReference,
        ));
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
    pub fn after(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        let query = reference.into().0.ok_or(Error::InvalidInput(
            crate::InvalidInput::UnusableMessageReference,
        ));
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
    pub fn around(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        let query = reference.into().0.ok_or(Error::InvalidInput(
            crate::InvalidInput::UnusableMessageReference,
        ));
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
    ) -> HistoryRequest {
        let start = start.into().0.ok_or(Error::InvalidInput(
            crate::InvalidInput::UnusableMessageReference,
        ));
        let query = start.and_then(|start| {
            end.into()
                .0
                .ok_or(Error::InvalidInput(
                    crate::InvalidInput::UnusableMessageReference,
                ))
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

impl From<&ChatMessage> for HistoryReference {
    fn from(value: &ChatMessage) -> Self {
        Self(value.msgid().map(|msgid| MsgRef::Msgid(msgid.to_owned())))
    }
}

/// A history request with a configurable page limit.
///
/// # Errors
///
/// Awaiting the request returns [`Error::Unsupported`] for [`crate::Feature::History`] with reason
/// [`crate::UnsupportedReason::NotOffered`] or
/// [`crate::UnsupportedReason::DisabledByClient`]. An unusable reference returns
/// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with [`crate::Refusal::Other`]. It
/// can also return [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
pub struct HistoryRequest {
    client: Client,
    query: Result<HistoryQuery, Error>,
}

impl HistoryRequest {
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

impl IntoFuture for HistoryRequest {
    type Output = Result<HistoryPage, Error>;
    type IntoFuture = crate::RequestFuture<HistoryPage>;

    fn into_future(self) -> Self::IntoFuture {
        match self.query {
            Ok(query) => {
                crate::RequestFuture::new(async move { self.client.history_page(query).await })
            }
            Err(error) => crate::RequestFuture::new(async move { Err(error) }),
        }
    }
}

impl Client {
    /// Starts a chat history request for `target`.
    #[must_use]
    pub fn history(&self, target: impl Into<crate::Target>) -> crate::History {
        crate::History {
            client: self.clone(),
            target: target.into().into_string(),
        }
    }

    pub(crate) async fn history_page(
        &self,
        query: HistoryQuery,
    ) -> Result<crate::HistoryPage, Error> {
        let target = match &query {
            HistoryQuery::Latest { target, .. }
            | HistoryQuery::Before { target, .. }
            | HistoryQuery::After { target, .. }
            | HistoryQuery::Around { target, .. }
            | HistoryQuery::Between { target, .. } => target.clone(),
            HistoryQuery::Targets { .. } | _ => String::new(),
        };
        if !target.is_empty() {
            crate::Target::from(target.clone())
                .check(self.state().isupport().inner())
                .map_err(|reason| crate::InvalidInput::invalid_name(target.clone(), reason))?;
        }
        let response = self.history_response(query).await?;
        let page = response.history.ok_or_else(Error::disconnected)?;
        let items = page
            .items
            .into_iter()
            .map(|item| history_item(item, self, crate::rt::now_wall(), Origin::History))
            .collect();
        Ok(crate::HistoryPage {
            target: target.into(),
            items,
            end: page.end,
        })
    }

    async fn history_response(&self, query: HistoryQuery) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::History { query, result })
            .await
            .map_err(|_| Error::disconnected())?;
        let response = receiver.await.map_err(|_| Error::disconnected())??;
        if let Some(error) = response.error() {
            return Err(Error::server(error));
        }
        Ok(response)
    }

    /// Lists targets that received messages during an inclusive time range.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::History`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn history_targets(
        &self,
        since: SystemTime,
        until: SystemTime,
    ) -> crate::RequestFuture<Vec<HistoryTarget>> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            client
                .history_response(HistoryQuery::Targets {
                    from: since,
                    to: until,
                    limit: 1_000,
                })
                .await?
                .history_targets
                .ok_or_else(Error::disconnected)
                .map(|targets| {
                    targets
                        .into_iter()
                        .map(|(target, latest)| HistoryTarget {
                            target: target.into(),
                            latest,
                        })
                        .collect()
                })
        })
    }

    /// Sets the server read marker for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::ReadMarkers`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. An unencodable timestamp returns
    /// [`Error::InvalidInput`]; a refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::TooManyPendingRequests`], [`Error::Timeout`],
    /// or [`Error::Disconnected`].
    pub fn mark_read(
        &self,
        target: impl Into<crate::Target>,
        time: SystemTime,
    ) -> crate::RequestFuture<Option<SystemTime>> {
        let client = self.clone();
        let target = target.into();
        crate::RequestFuture::new(async move {
            target
                .check(client.state().isupport().inner())
                .map_err(|error| crate::InvalidInput::InvalidMessage(error.to_string()))?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::MarkRead {
                    target: target.to_string(),
                    time,
                    result,
                })
                .await
                .map_err(|_| Error::disconnected())?;
            let response = receiver.await.map_err(|_| Error::disconnected())??;
            crate::requests::empty(&response)?;
            Ok(client.state().read_marker(&target))
        })
    }

    /// Queries the server read marker for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::ReadMarkers`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. It can also return
    /// [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`]; server refusals are classified as [`crate::Refusal::Other`].
    pub fn fetch_read_marker(
        &self,
        target: impl Into<crate::Target>,
    ) -> crate::RequestFuture<Option<SystemTime>> {
        let client = self.clone();
        let target = target.into();
        crate::RequestFuture::new(async move {
            target
                .check(client.state().isupport().inner())
                .map_err(|error| crate::InvalidInput::InvalidMessage(error.to_string()))?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::ReadMarker {
                    target: target.to_string(),
                    result,
                })
                .await
                .map_err(|_| Error::disconnected())?;
            receiver.await.map_err(|_| Error::disconnected())?
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::driver::test_support::*;
    use std::time::{Duration, UNIX_EPOCH};

    use papo_core::{HistoryMessage, HistoryQuery, MsgRef};
    use papo_proto::{LineRef, Message};

    use super::{HistoryItem, HistoryReference, history_item};

    fn core_item(message: &str) -> HistoryMessage {
        HistoryMessage {
            message: LineRef::parse(message).unwrap().to_owned(),
            context: false,
        }
    }

    #[test]
    fn converts_ids_timestamps_and_message_events() {
        assert!(
            matches!(HistoryReference::from("2Zl7JeYG6I8fG9q1lJ55BQ").0, Some(MsgRef::Msgid(id)) if id == "2Zl7JeYG6I8fG9q1lJ55BQ")
        );
        assert!(
            matches!(HistoryReference::from(UNIX_EPOCH).0, Some(MsgRef::Timestamp(time)) if time == UNIX_EPOCH)
        );
        let event = crate::Message::from_history(
            Message::privmsg("#papo", "earlier").with_tag("msgid", "RfcBTzyxY120004"),
            &crate::Client::disconnected_for_test(),
            UNIX_EPOCH,
        )
        .unwrap();
        assert!(
            matches!(HistoryReference::from(&event).0, Some(MsgRef::Msgid(id)) if id == "RfcBTzyxY120004")
        );
    }

    #[test]
    fn maps_reactions_and_redactions_from_history() {
        let client = crate::Client::disconnected_for_test();
        let reacted = history_item(
            papo_core::HistoryItem::Reacted(core_item(
                "@+draft/react=👍;+draft/reply=RfcBTzyxY120004 :emily!~e@localhost TAGMSG #papo",
            )),
            &client,
            UNIX_EPOCH,
            crate::Origin::History,
        );
        assert!(matches!(
            reacted,
            HistoryItem::Reacted(event)
                if event.source.as_ref().and_then(crate::Source::nick) == Some("emily")
                    && event.target == "#papo"
                    && event.message == "RfcBTzyxY120004"
                    && event.reaction == "👍"
                    && !event.removed
        ));

        let removal = history_item(
            papo_core::HistoryItem::Redacted(core_item(
                ":irc.example REDACT #papo RfcBTzyxY120004 :",
            )),
            &client,
            UNIX_EPOCH,
            crate::Origin::History,
        );
        assert!(matches!(
            removal,
            HistoryItem::Redacted(event)
                if event.target == "#papo"
                    && event.message == "RfcBTzyxY120004"
                    && event.reason.is_none()
        ));
    }

    #[test]
    fn malformed_typed_history_item_remains_raw() {
        let item = history_item(
            papo_core::HistoryItem::Redacted(core_item(":irc.example REDACT #papo")),
            &crate::Client::disconnected_for_test(),
            UNIX_EPOCH,
            crate::Origin::History,
        );
        assert!(matches!(item, HistoryItem::Other(line) if line.command() == "REDACT"));
    }

    #[test]
    fn maps_replayed_channel_activity() {
        let client = crate::Client::disconnected_for_test();
        let joined = history_item(
            papo_core::HistoryItem::Joined(core_item(
                ":emily!~e@localhost JOIN #papo emily :Emily Smith",
            )),
            &client,
            UNIX_EPOCH,
            crate::Origin::Replay,
        );
        assert!(matches!(
            joined,
            HistoryItem::Joined(event)
                if event.channel == "#papo" && event.nick == "emily" && !event.is_self
        ));

        let joined = history_item(
            papo_core::HistoryItem::Joined(core_item(
                ":PAPO!papo@localhost JOIN #papo papo :Papo client",
            )),
            &client,
            UNIX_EPOCH,
            crate::Origin::Replay,
        );
        assert!(matches!(joined, HistoryItem::Joined(event) if event.is_self));
    }

    #[test]
    fn maps_each_replayed_membership_change() {
        let client = crate::Client::disconnected_for_test();
        let map = |item| history_item(item, &client, UNIX_EPOCH, crate::Origin::Replay);

        let parted = map(papo_core::HistoryItem::Parted(core_item(
            ":emily!~e@localhost PART #papo :Leaving for lunch",
        )));
        assert!(matches!(
            parted,
            HistoryItem::Parted(event)
                if event.channel == "#papo" && event.nick == "emily"
                    && event.reason.as_deref() == Some("Leaving for lunch") && !event.is_self
        ));

        let quit = map(papo_core::HistoryItem::Quit(core_item(
            ":emily!~e@localhost QUIT :Connection reset by peer",
        )));
        assert!(matches!(
            quit,
            HistoryItem::Quit(event)
                if event.nick == "emily"
                    && event.reason.as_deref() == Some("Connection reset by peer")
                    && event.channels.is_empty()
        ));

        let kicked = map(papo_core::HistoryItem::Kicked(core_item(
            ":irc.example KICK #papo emily :Requested by alice",
        )));
        assert!(matches!(
            kicked,
            HistoryItem::Kicked(event)
                if event.channel == "#papo" && event.nick == "emily"
                    && event.reason.as_deref() == Some("Requested by alice")
                    && matches!(event.by, crate::Source::Server { ref name } if name == "irc.example")
                    && !event.is_self
        ));

        let renamed = map(papo_core::HistoryItem::NickChanged(core_item(
            ":emily!~e@localhost NICK emily_",
        )));
        assert!(matches!(
            renamed,
            HistoryItem::NickChanged(event)
                if event.old == "emily" && event.new == "emily_" && !event.is_self
        ));
    }

    #[test]
    fn maps_replayed_topic_modes_and_unreaction() {
        let client = crate::Client::disconnected_for_test();
        let map = |item| history_item(item, &client, UNIX_EPOCH, crate::Origin::Replay);

        let topic = map(papo_core::HistoryItem::TopicChanged(core_item(
            ":emily!~e@localhost TOPIC #papo :Rust 1.90 is out",
        )));
        assert!(matches!(
            topic,
            HistoryItem::TopicChanged(event)
                if event.channel == "#papo"
                    && event.topic.as_deref() == Some("Rust 1.90 is out")
                    && event.by.nick() == Some("emily")
        ));

        let mode = map(papo_core::HistoryItem::ModeChanged(core_item(
            ":emily!~e@localhost MODE #papo +im",
        )));
        assert!(matches!(
            mode,
            HistoryItem::ModeChanged(event)
                if event.target == "#papo" && event.by.nick() == Some("emily")
                    && event.changes.len() == 2
                    && event.changes.iter().all(|change| change.set)
                    && event.changes[0].mode == 'i' && event.changes[1].mode == 'm'
        ));

        let unreacted = map(papo_core::HistoryItem::Reacted(core_item(
            "@+draft/unreact=👍;+reply=RfcBTzyxY120004 :emily!~e@localhost TAGMSG #papo",
        )));
        assert!(matches!(
            unreacted,
            HistoryItem::Reacted(event) if event.reaction == "👍" && event.removed
        ));
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
    #[tokio::test(flavor = "current_thread")]
    async fn latest_history_resolves_with_history_messages() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        registration_burst(&mut server).await;
        server_line(
            &mut server,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(
            &mut server,
            b"CAP REQ :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        server_line(
            &mut server,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch server-time\r\n",
        )
        .await;
        client_line(&mut server, b"CAP END\r\n").await;
        server_line(&mut server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(events.next().await, Some(Event::Registered(_))));
        assert!(matches!(
            client.history("#papo invalid").latest(10).await,
            Err(Error::InvalidInput(crate::InvalidInput::InvalidName { name, .. }))
                if name == "#papo invalid"
        ));
        let request = tokio::spawn(async move { client.history("#papo").latest(10).await });
        client_line(&mut server, b"@label=0 CHATHISTORY LATEST #papo * 10\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +reply labeled-response\r\n@batch=reply;draft/chathistory-end :ergo.test BATCH +history chathistory #papo\r\n@batch=history;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120004 :emily!~e@localhost PRIVMSG #papo :earlier\r\n@batch=history;+draft/react=eyes;+reply=RfcBTzyxY120004 :emily!~e@localhost TAGMSG #papo\r\n@batch=history :emily!~e@localhost JOIN #papo emily :Emily Smith\r\n@batch=history :emily!~e@localhost TAGMSG #papo\r\n@batch=reply :ergo.test BATCH -history\r\n:ergo.test BATCH -reply\r\n").await;
        let page = within(request).await.unwrap().unwrap();
        assert!(page.end);
        assert_eq!(page.items.len(), 4);
        assert!(matches!(page.items[1], HistoryItem::Reacted(_)));
        assert!(matches!(page.items[2], HistoryItem::Joined(_)));
        assert!(matches!(page.items[3], HistoryItem::Other(_)));
        let message = page.messages().next().unwrap();
        assert_eq!(message.text(), "earlier");
        assert_eq!(message.origin(), Origin::History);
        assert!(!message.is_live());
        let Some(Event::History(history)) = events.next().await else {
            panic!("expected replay batch event")
        };
        assert!(matches!(history.items[1], HistoryItem::Reacted(_)));
        assert!(matches!(history.items[2], HistoryItem::Joined(_)));
        assert!(matches!(history.items[3], HistoryItem::Other(_)));
        assert_eq!(history.messages().next().unwrap().origin(), Origin::Replay);
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reference_history_queries_reach_the_wire_and_resolve() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_history_registration(&mut server, &mut events).await;

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("#papo")
                    .before("RfcBTzyxY119999")
                    .limit(50)
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 CHATHISTORY BEFORE #papo msgid=RfcBTzyxY119999 50\r\n",
        )
        .await;
        history_response(&mut server, 0).await;
        let before = within(request).await.unwrap().unwrap();

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("#papo")
                    .after(UNIX_EPOCH + Duration::from_secs(42))
                    .limit(51)
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=1 CHATHISTORY AFTER #papo timestamp=1970-01-01T00:00:42.000Z 51\r\n",
        )
        .await;
        history_response(&mut server, 1).await;
        within(request).await.unwrap().unwrap();

        let message = before.messages().next().unwrap().clone();
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.history("#papo").around(&message).limit(52).await }
        });
        client_line(
            &mut server,
            b"@label=2 CHATHISTORY AROUND #papo msgid=RfcBTzyxY120000 52\r\n",
        )
        .await;
        history_response(&mut server, 2).await;
        within(request).await.unwrap().unwrap();

        let request = tokio::spawn(async move {
            client
                .history("#papo")
                .between("RfcBTzyxY119998", "RfcBTzyxY120003")
                .limit(53)
                .await
        });
        client_line(
            &mut server,
            b"@label=3 CHATHISTORY BETWEEN #papo msgid=RfcBTzyxY119998 msgid=RfcBTzyxY120003 53\r\n",
        )
        .await;
        history_response(&mut server, 3).await;
        within(request).await.unwrap().unwrap();
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn history_fails_without_the_negotiated_capability() {
        let (mut server, client, mut events, driver) = scripted(config());
        let task = tokio::spawn(driver);
        complete_registration(&mut server, &mut events).await;
        assert!(matches!(
            within(async { client.history("#papo").latest(10).await }).await,
            Err(Error::Unsupported(crate::Unsupported {
                feature: crate::Feature::History,
                reason: crate::UnsupportedReason::NotOffered,
                ..
            }))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn history_targets_are_typed_and_time_bounded() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_history_registration(&mut server, &mut events).await;
        let request = tokio::spawn(async move {
            client
                .history_targets(
                    UNIX_EPOCH + Duration::from_secs(42),
                    UNIX_EPOCH + Duration::from_secs(84),
                )
                .await
        });
        client_line(
            &mut server,
            b"@label=0 CHATHISTORY TARGETS timestamp=1970-01-01T00:00:42.000Z timestamp=1970-01-01T00:01:24.000Z 100\r\n",
        )
        .await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +reply labeled-response\r\n@batch=reply :ergo.test BATCH +targets draft/chathistory-targets\r\n@batch=targets :ergo.test CHATHISTORY TARGETS #papo 2026-10-15T12:00:00.000Z\r\n@batch=reply :ergo.test BATCH -targets\r\n:ergo.test BATCH -reply\r\n").await;
        let targets = within(request).await.unwrap().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].target, "#papo");
        assert_eq!(
            targets[0].latest,
            UNIX_EPOCH + Duration::from_hours(497_796)
        );
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mark_read_sets_a_millisecond_server_timestamp() {
        let config = config().wanted_caps(["draft/read-marker", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_read_marker_registration(&mut server, &mut events).await;
        let request = tokio::spawn(async move {
            client
                .mark_read(
                    "#papo",
                    UNIX_EPOCH + Duration::new(1_760_529_342, 123_999_999),
                )
                .await
        });
        client_line(
            &mut server,
            b"@label=0 MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        assert_eq!(
            within(request).await.unwrap().unwrap(),
            Some(UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000))
        );
        assert!(
            matches!(events.next().await, Some(Event::ReadMarker(marker)) if marker.target == "#papo")
        );
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn read_marker_resolves_fresh_and_cached_values() {
        let config = config().wanted_caps(["draft/read-marker", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_read_marker_registration(&mut server, &mut events).await;
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.fetch_read_marker("#papo").await }
        });
        client_line(&mut server, b"MARKREAD #papo\r\n").await;
        server_line(
            &mut server,
            b":ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        let expected = UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000);
        assert_eq!(within(request).await.unwrap().unwrap(), Some(expected));
        assert_eq!(client.state().read_marker("#PAPO"), Some(expected));

        assert_eq!(
            within(client.fetch_read_marker("#papo")).await.unwrap(),
            Some(expected)
        );
        client_line(&mut server, b"MARKREAD #papo\r\n").await;
        abort_task(task).await;
    }
}
