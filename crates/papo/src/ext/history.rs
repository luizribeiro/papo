use std::{future::IntoFuture, time::SystemTime};

use papo_core::{HistoryQuery, MsgRef, Response};
use papo_proto::{ModeChanges, Reaction};
use tokio::sync::oneshot;

use crate::{
    Client, Error, Event, Line, Message,
    driver::Command,
    event::{
        Kicked, ModeChanged, NickChanged, Parted, Quit, Reacted, Redacted, TagMsg, TopicChanged,
    },
};

/// Result of moving a conversation's server-side read marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MarkReadOutcome {
    /// The marker advanced to this server timestamp.
    Advanced(SystemTime),
    /// The marker was already at or beyond the requested timestamp.
    #[non_exhaustive]
    Unchanged {
        /// The marker retained by the server.
        stored: SystemTime,
    },
    /// The server retained a marker newer than the newest line known to this client.
    #[non_exhaustive]
    BehindLatest {
        /// The marker retained by the server.
        stored: SystemTime,
        /// The newest line this client asked to mark as read.
        latest: SystemTime,
    },
}

/// Builds a request to move a conversation's read marker using server time.
#[must_use = "requests do nothing unless awaited"]
pub struct MarkReadRequest {
    client: Client,
    target: crate::target::sealed::Parts<'static>,
    message: Option<crate::MessageRef>,
}

impl std::fmt::Debug for MarkReadRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("MarkReadRequest", formatter)
    }
}

impl MarkReadRequest {
    /// Marks the conversation read through this received message's server timestamp.
    ///
    /// Awaiting returns [`crate::InvalidInput::WrongConversation`] if the message belongs to a
    /// different conversation, and [`crate::InvalidInput::NoServerTime`] if it has no server time.
    pub fn up_to(mut self, message: &crate::MessageRef) -> Self {
        self.message = Some(message.clone());
        self
    }
}

impl IntoFuture for MarkReadRequest {
    type Output = Result<MarkReadOutcome, Error>;
    type IntoFuture = crate::RequestFuture<MarkReadOutcome>;

    fn into_future(self) -> Self::IntoFuture {
        crate::RequestFuture::new(async move {
            let target = self.client.resolve(
                crate::target::Role::Conversation("mark_read: conversation"),
                self.target,
            )?;
            let latest = if let Some(message) = self.message {
                let server_time = message
                    .server_time()
                    .ok_or(crate::InvalidInput::NoServerTime)?;
                let conversation = message
                    .conversation()
                    .ok_or(crate::InvalidInput::WrongConversation)?;
                let message_target = self.client.resolve(
                    crate::target::Role::Conversation("mark_read: message"),
                    crate::AsConversation::__parts(conversation),
                )?;
                if !self.client.state().same_name(&target, &message_target) {
                    return Err(crate::InvalidInput::WrongConversation.into());
                }
                server_time
            } else {
                self.client
                    .state()
                    .latest_server_time(&target)
                    .ok_or(crate::InvalidInput::NoServerTime)?
            };
            let previous = read_marker_time(self.client.state().read_marker(&target));
            let (result, receiver) = oneshot::channel();
            self.client
                .commands
                .send(Command::MarkRead {
                    target: target.clone(),
                    time: latest,
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            let response = receiver.await.map_err(|_| Error::connection_lost())??;
            crate::requests::empty(&response)?;
            let stored = read_marker_time(self.client.state().read_marker(&target))
                .ok_or(Error::Unconfirmed)?;
            if stored > latest {
                Ok(MarkReadOutcome::BehindLatest { stored, latest })
            } else if stored < latest {
                Err(Error::Unconfirmed)
            } else if previous.is_some_and(|previous| previous >= latest) {
                Ok(MarkReadOutcome::Unchanged { stored })
            } else {
                Ok(MarkReadOutcome::Advanced(stored))
            }
        })
    }
}

fn other(line: Line, received_at: SystemTime) -> Event {
    crate::event_impl::unparsed_from_line(line, received_at)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn history_item(
    item: papo_core::HistoryItem,
    history_conversation: Option<&crate::Conversation>,
    client: &Client,
    received_at: SystemTime,
) -> Event {
    let state = client.state();
    let mapping = state.casemapping();
    let origin = client.origin();
    let account_tag_enabled = state.caps().contains_key(&crate::cap::ACCOUNT_TAG);
    let prove_not_bot = crate::event_impl::can_prove_non_bot(
        state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
        state.isupport().bot_mode().is_some(),
    );
    let mut event = match item {
        papo_core::HistoryItem::Message(item) => {
            let authorship = crate::Authorship::from_core(item.authorship);
            crate::event_impl::historical_message_event(
                item.line,
                authorship,
                history_conversation,
                client,
                received_at,
            )
        }
        papo_core::HistoryItem::Incomplete(item) => {
            let authorship = crate::Authorship::from_core(item.authorship);
            let raw = item.line.clone();
            let event = crate::event_impl::historical_message_event(
                item.line,
                authorship,
                history_conversation,
                client,
                received_at,
            );
            crate::event_impl::mark_incomplete(event).unwrap_or_else(|| other(raw, received_at))
        }
        papo_core::HistoryItem::Reacted(item) => typed_or_raw(&item.line, received_at, |message| {
            reaction(
                message,
                crate::Authorship::from_core(item.authorship),
                mapping,
                origin,
                state.nick(),
                state.isupport().inner(),
                history_conversation,
                account_tag_enabled,
                prove_not_bot,
                received_at,
                Some(&state),
            )
        }),
        papo_core::HistoryItem::Redacted(item) => {
            typed_or_raw(&item.line, received_at, |message| {
                redaction(
                    message,
                    crate::Authorship::from_core(item.authorship),
                    mapping,
                    origin,
                    state.nick(),
                    state.isupport().inner(),
                    history_conversation,
                    account_tag_enabled,
                    prove_not_bot,
                    received_at,
                    Some(&state),
                )
            })
        }
        papo_core::HistoryItem::Joined(item) => typed_or_raw(&item.line, received_at, |message| {
            joined(
                message,
                crate::Authorship::from_core(item.authorship),
                client,
                received_at,
            )
        }),
        papo_core::HistoryItem::Parted(item) => typed_or_raw(&item.line, received_at, |message| {
            parted(
                message,
                crate::Authorship::from_core(item.authorship),
                client,
                received_at,
            )
        }),
        papo_core::HistoryItem::Quit(item) => typed_or_raw(&item.line, received_at, |message| {
            quit(
                message,
                crate::Authorship::from_core(item.authorship),
                history_conversation,
                client,
                received_at,
            )
        }),
        papo_core::HistoryItem::Kicked(item) => typed_or_raw(&item.line, received_at, |message| {
            kicked(
                message,
                crate::Authorship::from_core(item.authorship),
                crate::Authorship::from_core(item.subject_authorship),
                client,
                received_at,
            )
        }),
        papo_core::HistoryItem::NickChanged(item) => {
            typed_or_raw(&item.line, received_at, |message| {
                nick_changed(
                    message,
                    crate::Authorship::from_core(item.authorship),
                    client,
                    history_conversation,
                    received_at,
                )
            })
        }
        papo_core::HistoryItem::TopicChanged(item) => {
            typed_or_raw(&item.line, received_at, |message| {
                topic_changed(
                    message,
                    crate::Authorship::from_core(item.authorship),
                    mapping,
                    origin,
                    account_tag_enabled,
                    prove_not_bot,
                    received_at,
                )
            })
        }
        papo_core::HistoryItem::ModeChanged(item) => {
            typed_or_raw(&item.line, received_at, |message| {
                mode_changed(
                    message,
                    crate::Authorship::from_core(item.authorship),
                    crate::Authorship::from_core(item.subject_authorship),
                    client,
                    received_at,
                )
            })
        }
        papo_core::HistoryItem::TagMsg(item) => typed_or_raw(&item.line, received_at, |message| {
            historical_tagmsg(
                message,
                crate::Authorship::from_core(item.authorship),
                history_conversation,
                client,
                received_at,
            )
        }),
        item => {
            let raw = item.into_line().line;
            other(raw, received_at)
        }
    };
    crate::event_impl::attach_historical_peer_key(&mut event, &state);
    event
}

fn historical_tagmsg(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    history_conversation: Option<&crate::Conversation>,
    client: &Client,
    received_at: SystemTime,
) -> Option<Event> {
    if !message.command_is("TAGMSG") {
        return None;
    }
    let state = client.state();
    let mapping = state.casemapping();
    let (status, conversation) =
        papo_core::classify_conversation(message, state.nick().as_str(), state.isupport().inner())
            .ok()?;
    let conversation = crate::event_impl::historical_conversation(
        message,
        authorship,
        conversation,
        history_conversation,
        state.nick(),
        mapping,
    );
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        client.origin(),
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        received_at,
    );
    let actor = user_from_line(message, &envelope, mapping, client.origin(), false)?;
    let mut receipt = crate::message_ref::Receipt::new(
        crate::Source::User(actor.clone()),
        authorship,
        &conversation,
        None,
        crate::event_impl::return_address(message, &conversation),
    );
    receipt.set_reference(envelope.msgid.iter().cloned(), None);
    Some(Event::TagMsg(TagMsg {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        actor,
        authorship,
        conversation,
        status,
        receipt,
        tags: crate::Tags::from_message(message),
    }))
}

fn typed_or_raw<F>(raw: &papo_proto::Line, received_at: SystemTime, convert: F) -> Event
where
    F: FnOnce(&papo_proto::LineRef<'_>) -> Option<Event>,
{
    convert(&raw.as_ref()).unwrap_or_else(|| other(raw.clone(), received_at))
}

fn actor_from_line(
    message: &papo_proto::LineRef<'_>,
    envelope: &crate::event_impl::EventEnvelope,
    mapping: crate::Casemapping,
    origin: crate::Origin,
    prove_not_bot: bool,
) -> Option<crate::Source> {
    crate::event_impl::source::from_line(
        crate::event_impl::source::LinePrefix::Raw(message.raw_source()?),
        envelope,
        mapping,
        origin,
        prove_not_bot,
    )
}

fn user_from_line(
    message: &papo_proto::LineRef<'_>,
    envelope: &crate::event_impl::EventEnvelope,
    mapping: crate::Casemapping,
    origin: crate::Origin,
    prove_not_bot: bool,
) -> Option<crate::UserSource> {
    match actor_from_line(message, envelope, mapping, origin, prove_not_bot)? {
        crate::Source::User(user) => Some(user),
        crate::Source::Server(_) => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn reaction(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    mapping: crate::Casemapping,
    origin: crate::Origin,
    own_nick: &crate::Nick,
    isupport: &papo_proto::Isupport,
    history_conversation: Option<&crate::Conversation>,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    state: Option<&crate::State>,
) -> Option<Event> {
    let reaction = Reaction::parse(message)?;
    let (status, conversation) =
        papo_core::classify_conversation(message, own_nick.as_str(), isupport).ok()?;
    let conversation = crate::event_impl::historical_conversation(
        message,
        authorship,
        conversation,
        history_conversation,
        own_nick,
        mapping,
    );
    let reply_to = reaction.reply_to.clone();
    let (reaction, removed) = reaction
        .react
        .map(|value| (value, false))
        .or_else(|| reaction.unreact.map(|value| (value, true)))?;
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        origin,
        account_tag_enabled,
        received_at,
    );
    let return_address = crate::event_impl::return_address(message, &conversation);
    let actor = user_from_line(message, &envelope, mapping, origin, prove_not_bot)?;
    let mut receipt = crate::message_ref::Receipt::new(
        crate::Source::User(actor.clone()),
        authorship,
        &conversation,
        state.and_then(|state| state.reply_token(&conversation)),
        return_address,
    );
    receipt.set_channel(state.and_then(|state| state.channel_guard(&conversation)));
    Some(Event::Reacted(Reacted {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        actor,
        authorship,
        conversation,
        status,
        receipt,
        reacts_to: reply_to.into(),
        reaction,
        action: if removed {
            crate::event::ReactionAction::Removed
        } else {
            crate::event::ReactionAction::Added
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn redaction(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    mapping: crate::Casemapping,
    origin: crate::Origin,
    own_nick: &crate::Nick,
    isupport: &papo_proto::Isupport,
    history_conversation: Option<&crate::Conversation>,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
    state: Option<&crate::State>,
) -> Option<Event> {
    let (status, conversation) =
        papo_core::classify_conversation(message, own_nick.as_str(), isupport).ok()?;
    let conversation = crate::event_impl::historical_conversation(
        message,
        authorship,
        conversation,
        history_conversation,
        own_nick,
        mapping,
    );
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        origin,
        account_tag_enabled,
        received_at,
    );
    let return_address = crate::event_impl::return_address(message, &conversation);
    let actor = actor_from_line(message, &envelope, mapping, origin, prove_not_bot)?;
    let mut receipt = crate::message_ref::Receipt::new(
        actor.clone(),
        authorship,
        &conversation,
        state.and_then(|state| state.reply_token(&conversation)),
        return_address,
    );
    receipt.set_channel(state.and_then(|state| state.channel_guard(&conversation)));
    Some(Event::Redacted(Redacted {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        actor,
        authorship,
        conversation,
        status,
        receipt,
        redacts: message.param(1)?.into(),
        reason: message
            .param(2)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned),
    }))
}

fn joined(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    client: &Client,
    received_at: SystemTime,
) -> Option<Event> {
    let state = client.state();
    let mapping = state.casemapping();
    crate::event_impl::joined_from_line(
        message,
        authorship,
        mapping,
        client.origin(),
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        crate::event_impl::can_prove_non_bot(
            state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
            state.isupport().bot_mode().is_some(),
        ),
        received_at,
    )
    .map(Event::Joined)
}

fn parted(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    client: &Client,
    received_at: SystemTime,
) -> Option<Event> {
    let state = client.state();
    let mapping = state.casemapping();
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        client.origin(),
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        received_at,
    );
    let actor = user_from_line(
        message,
        &envelope,
        mapping,
        client.origin(),
        crate::event_impl::can_prove_non_bot(
            state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
            state.isupport().bot_mode().is_some(),
        ),
    )?;
    Some(Event::Parted(Parted {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        actor,
        channel: state.known_name_rules()?.channel(message.param(0)?),
        authorship,
        reason: message
            .param(1)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned),
    }))
}

fn quit(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    history_conversation: Option<&crate::Conversation>,
    client: &Client,
    received_at: SystemTime,
) -> Option<Event> {
    let state = client.state();
    let mapping = state.casemapping();
    let origin = client.origin();
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        origin,
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        received_at,
    );
    let actor = user_from_line(
        message,
        &envelope,
        mapping,
        origin,
        crate::event_impl::can_prove_non_bot(
            state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
            state.isupport().bot_mode().is_some(),
        ),
    )?;
    Some(Event::Quit(Quit {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        actor,
        reason: message
            .param(0)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned),
        channels: match history_conversation {
            Some(crate::Conversation::Channel(channel)) => vec![channel.clone()],
            _ => Vec::new(),
        },
        authorship,
        netsplit: None,
    }))
}

fn kicked(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    subject_authorship: crate::Authorship,
    client: &Client,
    received_at: SystemTime,
) -> Option<Event> {
    let nick = message.param(1)?.to_owned();
    let state = client.state();
    let mapping = state.casemapping();
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        client.origin(),
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        received_at,
    );
    let actor = actor_from_line(
        message,
        &envelope,
        mapping,
        client.origin(),
        crate::event_impl::can_prove_non_bot(
            state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
            state.isupport().bot_mode().is_some(),
        ),
    )?;
    Some(Event::Kicked(Kicked {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        channel: state.known_name_rules()?.channel(message.param(0)?),
        authorship,
        subject_authorship,
        subject: state.known_name_rules()?.nick(nick),
        actor,
        reason: message
            .param(2)
            .filter(|reason| !reason.is_empty())
            .map(str::to_owned),
    }))
}

fn nick_changed(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    client: &Client,
    history_conversation: Option<&crate::Conversation>,
    received_at: SystemTime,
) -> Option<Event> {
    source_nick(message)?;
    let state = client.state();
    let mapping = state.casemapping();
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        client.origin(),
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        received_at,
    );
    let actor = user_from_line(
        message,
        &envelope,
        mapping,
        client.origin(),
        crate::event_impl::can_prove_non_bot(
            state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
            state.isupport().bot_mode().is_some(),
        ),
    )?;
    Some(Event::NickChanged(NickChanged {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        actor,
        authorship,
        new: state.known_name_rules()?.nick(message.param(0)?),
        channels: match history_conversation {
            Some(crate::Conversation::Channel(channel)) => vec![channel.clone()],
            _ => Vec::new(),
        },
    }))
}

fn topic_changed(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    mapping: crate::Casemapping,
    origin: crate::Origin,
    account_tag_enabled: bool,
    prove_not_bot: bool,
    received_at: SystemTime,
) -> Option<Event> {
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        origin,
        account_tag_enabled,
        received_at,
    );
    let actor = actor_from_line(message, &envelope, mapping, origin, prove_not_bot)?;
    Some(Event::TopicChanged(TopicChanged {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        channel: origin.__channel(mapping, message.param(0)?),
        topic: message
            .param(1)
            .filter(|topic| !topic.is_empty())
            .map(str::to_owned),
        actor,
        authorship,
    }))
}

fn mode_changed(
    message: &papo_proto::LineRef<'_>,
    authorship: crate::Authorship,
    _subject_authorship: crate::Authorship,
    client: &Client,
    received_at: SystemTime,
) -> Option<Event> {
    let state = client.state();
    let mapping = state.casemapping();
    let envelope = crate::event_impl::line_envelope_with_account_tag(
        message,
        mapping,
        client.origin(),
        state.caps().contains_key(&crate::cap::ACCOUNT_TAG),
        received_at,
    );
    let changes = ModeChanges::parse(
        message.param(1)?,
        message.params().skip(2),
        state.isupport().inner(),
    )
    .map(|change| {
        crate::event_impl::typed_mode_change(
            change.action,
            change.mode,
            change.arg,
            change.kind,
            mapping,
            client.origin(),
        )
    })
    .collect();
    let actor = actor_from_line(
        message,
        &envelope,
        mapping,
        client.origin(),
        crate::event_impl::can_prove_non_bot(
            state.caps().contains_key(&crate::cap::MESSAGE_TAGS),
            state.isupport().bot_mode().is_some(),
        ),
    )?;
    Some(Event::ModeChanged(ModeChanged {
        msgid: envelope.msgid,
        server_time: envelope.server_time,
        received_at: envelope.received_at,
        subject: if state.isupport().inner().is_channel(message.param(0)?) {
            crate::Subject::Channel(state.known_name_rules()?.channel(message.param(0)?))
        } else {
            crate::Subject::User(state.known_name_rules()?.nick(message.param(0)?))
        },
        actor,
        authorship,
        changes,
        own_nick: None,
    }))
}

fn source_nick(message: &papo_proto::LineRef<'_>) -> Option<String> {
    match message.source()? {
        papo_proto::Source::User { nick, .. } => Some(nick.to_owned()),
        papo_proto::Source::Server(_) => None,
    }
}

/// Why a history page was produced.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum HistoryReason {
    /// The page completed an explicit [`Client::history`] request.
    Requested,
    /// The page fills a gap after reconnecting.
    Catchup,
    /// The server sent the page without an active history request.
    Replay,
}

/// A page of requested or server-initiated chat history.
///
/// Event streams deliver a complete page when its batch closes. Applications should insert its
/// items by server time rather than assuming the page arrived before later live events.
/// Items remain in server delivery order. `IRCv3` recommends oldest-to-newest delivery but
/// permits other orders, so papo does not sort the page.
///
/// Select the chronologically oldest chat message explicitly:
///
/// ```
/// # fn oldest(page: &papo::HistoryPage) {
/// let oldest = page.chat_messages().min_by_key(|message| {
///     message.server_time.unwrap_or(message.received_at)
/// });
/// # let _ = oldest;
/// # }
/// ```
///
/// When `server_time` is missing, `received_at` provides a local fallback; the message's true
/// historical position is unknown.
///
/// A page cursor can be reused to continue in either direction:
///
/// ```no_run
/// # async fn show_bounds(client: &papo::Client) -> Result<(), papo::Error> {
/// let page = client.history("#papo").latest().await?;
/// if let Some(cursor) = page.cursor() {
///     let previous = client.history("#papo").before(cursor.clone()).await?;
///     let following = client.history("#papo").after(cursor).await?;
///     println!("{} previous items; {} following items", previous.items.len(), following.items.len());
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HistoryPage {
    /// Conversation whose history was requested.
    pub conversation: crate::Conversation,
    /// Replayed entries in server delivery order, normally oldest to newest.
    ///
    /// The server is allowed to use a different order; papo does not sort the entries.
    pub items: Vec<Event>,
    /// Why this page was produced.
    pub reason: HistoryReason,
    /// Whether this page was empty or the server reported that no further page exists.
    ///
    /// For [`History::latest_after`], `true` means every available item after the reference was
    /// returned. When it is `false`, the oldest available items may have been omitted between the
    /// reference and this page. A page shorter than the requested limit does not imply the end,
    /// because the server may cap or vary the number of returned items.
    pub end: bool,
}

/// A conversation with the timestamp of its newest available history entry.
///
/// Returned by the `IRCv3` [`CHATHISTORY TARGETS`] query.
///
/// [`CHATHISTORY TARGETS`]: https://ircv3.net/specs/extensions/chathistory#targets
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RecentConversation {
    /// Conversation with available history.
    pub conversation: crate::Conversation,
    /// Server timestamp of the newest available entry.
    pub latest_server_time: SystemTime,
}

impl HistoryPage {
    /// Which `Client` produced this value (not the sender or the server).
    #[must_use]
    pub fn origin(&self) -> crate::Origin {
        crate::message_ref::conversation_origin(&self.conversation)
    }

    /// Iterates over complete `PRIVMSG` chat messages, including own messages, in item order.
    ///
    /// A user interface should render [`HistoryPage::items`] so notices and activity are not
    /// dropped. Incomplete multiline items are available only through `items`. This iterator
    /// remains in server delivery order.
    pub fn chat_messages(&self) -> impl Iterator<Item = &Message> {
        self.items.iter().filter_map(|item| match item {
            Event::Message(message) => Some(message),
            _ => None,
        })
    }

    /// Returns a cursor that continues from either edge of this page.
    ///
    /// [`History::before`] continues from the first delivered item. [`History::after`] and
    /// [`History::latest_after`] continue from the last. If the server does not follow the
    /// recommended oldest-to-newest order, those items are not necessarily chronological edges.
    #[must_use]
    pub fn cursor(&self) -> Option<HistoryReference> {
        let origin = crate::message_ref::conversation_origin(&self.conversation);
        let value = event_reference(self.items.first()?)?;
        let after = event_reference(self.items.last()?)?;
        Some(HistoryReference {
            after: (after != value).then_some(after),
            value,
            origin: (origin != crate::Origin::NONE).then_some(origin),
        })
    }
}

impl RecentConversation {
    /// Which `Client` produced this value (not the sender or the server).
    #[must_use]
    pub fn origin(&self) -> crate::Origin {
        crate::message_ref::conversation_origin(&self.conversation)
    }
}

fn event_reference(event: &Event) -> Option<MsgRef> {
    event.msgid().map_or_else(
        || event.server_time().map(MsgRef::Timestamp),
        |msgid| Some(MsgRef::Msgid(msgid.to_string())),
    )
}

/// Builds a history request for one channel or nickname.
#[must_use = "requests do nothing unless awaited"]
pub struct History {
    pub(crate) client: Client,
    pub(crate) target: Result<String, Error>,
}

impl std::fmt::Debug for History {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("History", formatter)
    }
}

const DEFAULT_HISTORY_LIMIT: usize = 100;

impl History {
    /// Requests the newest messages.
    ///
    /// The request defaults to 100 messages. Use [`HistoryRequest::limit`] to change it; the
    /// server may cap the requested limit lower.
    pub fn latest(self) -> HistoryRequest {
        HistoryRequest {
            client: self.client,
            query: self
                .target
                .map(|target| HistoryQuery::__latest(target, None, DEFAULT_HISTORY_LIMIT)),
        }
    }

    /// Requests the newest messages strictly after `reference`, at most the requested limit.
    ///
    /// If more messages exist, the oldest are omitted, so a full page may hide a gap between
    /// `reference` and the page; check [`HistoryPage::end`] to learn whether the page is complete.
    /// Unlike [`History::after`], this returns the newest page after the reference.
    pub fn latest_after(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        self.reference_query(reference, CursorEdge::After, |target, after, limit| {
            HistoryQuery::__latest(target, Some(after), limit)
        })
    }

    /// Requests messages immediately before `reference`.
    pub fn before(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        self.reference_query(reference, CursorEdge::Before, HistoryQuery::__before)
    }

    /// Requests the oldest messages strictly after `reference`, at most the requested limit.
    ///
    /// Unlike [`History::latest_after`], this returns the oldest page after the reference.
    pub fn after(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        self.reference_query(reference, CursorEdge::After, HistoryQuery::__after)
    }

    /// Requests messages surrounding `reference`.
    pub fn around(self, reference: impl Into<HistoryReference>) -> HistoryRequest {
        self.reference_query(reference, CursorEdge::After, HistoryQuery::__around)
    }

    fn reference_query(
        self,
        reference: impl Into<HistoryReference>,
        edge: CursorEdge,
        build: impl FnOnce(String, MsgRef, usize) -> HistoryQuery,
    ) -> HistoryRequest {
        let reference = reference.into();
        let origin = reference.origin;
        let value = reference.edge(edge);
        let query = self
            .target
            .and_then(|target| resolve_reference(&self.client, target, origin))
            .map(|target| build(target, value, DEFAULT_HISTORY_LIMIT));
        HistoryRequest {
            client: self.client,
            query,
        }
    }

    /// Requests messages between two exclusive references.
    pub fn between(
        self,
        start: impl Into<HistoryReference>,
        end: impl Into<HistoryReference>,
    ) -> HistoryRequest {
        let start = start.into();
        let end = end.into();
        let query = self
            .target
            .and_then(|target| resolve_reference(&self.client, target, start.origin))
            .and_then(|target| resolve_reference(&self.client, target, end.origin))
            .map(|target| {
                let start = start.edge(CursorEdge::After);
                let end = end.edge(CursorEdge::Before);
                HistoryQuery::__between(target, start, end, DEFAULT_HISTORY_LIMIT)
            });
        HistoryRequest {
            client: self.client,
            query,
        }
    }
}

fn resolve_reference(
    client: &Client,
    target: String,
    origin: Option<crate::Origin>,
) -> Result<String, Error> {
    client.resolve(
        crate::target::Role::Conversation("history: conversation"),
        crate::target::sealed::Parts {
            text: std::borrow::Cow::Owned(target),
            origin: origin.unwrap_or(crate::Origin::NONE),
            recipient: None,
            channel: None,
        },
    )
}

/// An exclusive position used to continue a history request.
///
/// A cursor from [`HistoryPage::cursor`] retains both page edges. Cursors made from a message ID
/// or timestamp use that position for both directions. Timestamp cursors can skip other lines
/// carrying the same timestamp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryReference {
    value: MsgRef,
    after: Option<MsgRef>,
    origin: Option<crate::Origin>,
}

#[derive(Clone, Copy)]
enum CursorEdge {
    Before,
    After,
}

impl HistoryReference {
    fn new(value: MsgRef, origin: Option<crate::Origin>) -> Self {
        Self {
            value,
            after: None,
            origin,
        }
    }

    fn edge(self, edge: CursorEdge) -> MsgRef {
        match edge {
            CursorEdge::Before => self.value,
            CursorEdge::After => self.after.unwrap_or(self.value),
        }
    }
}

impl From<crate::MessageId> for HistoryReference {
    fn from(value: crate::MessageId) -> Self {
        Self::new(MsgRef::Msgid(value.to_string()), None)
    }
}

impl From<&crate::MessageId> for HistoryReference {
    fn from(value: &crate::MessageId) -> Self {
        Self::new(MsgRef::Msgid(value.to_string()), None)
    }
}

impl From<SystemTime> for HistoryReference {
    fn from(value: SystemTime) -> Self {
        Self::new(MsgRef::Timestamp(value), None)
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
#[must_use = "requests do nothing unless awaited"]
pub struct HistoryRequest {
    client: Client,
    query: Result<HistoryQuery, Error>,
}

impl std::fmt::Debug for HistoryRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("HistoryRequest", formatter)
    }
}

impl HistoryRequest {
    /// Sets the requested maximum number of returned messages.
    ///
    /// The default is 100. The server may cap the requested limit lower.
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
    /// Starts a chat history request for a conversation.
    ///
    /// The returned page keeps server delivery order. `IRCv3` recommends oldest-to-newest
    /// delivery for every history subcommand but permits other orders; papo does not sort it.
    pub fn history(&self, target: impl crate::AsConversation) -> crate::History {
        crate::History {
            client: self.clone(),
            target: self.resolve(
                crate::target::Role::Conversation("history: conversation"),
                crate::AsConversation::__parts(&target),
            ),
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
            self.state()
                .parse_target_inner(&target)
                .map_err(|reason| crate::InvalidInput::invalid_name(target.clone(), reason))?;
        }
        let response = self.history_response(query).await?;
        let page = response.history.ok_or_else(Error::connection_lost)?;
        let papo_core::HistoryPage {
            conversation,
            items,
            end,
        } = page;
        let conversation = crate::message_ref::stamp_conversation(conversation, self.origin());
        let items = items
            .into_iter()
            .map(|item| history_item(item, Some(&conversation), self, crate::rt::now_wall()))
            .collect();
        Ok(crate::HistoryPage {
            conversation,
            items,
            end,
            reason: HistoryReason::Requested,
        })
    }

    async fn history_response(&self, query: HistoryQuery) -> Result<Response, Error> {
        let (result, receiver) = oneshot::channel();
        self.commands
            .send(Command::History { query, result })
            .await
            .map_err(|_| Error::connection_lost())?;
        let response = receiver.await.map_err(|_| Error::connection_lost())??;
        if let Some(error) = response.error() {
            return Err(Error::server(error));
        }
        Ok(response)
    }

    /// Lists conversations that received messages during an inclusive time range.
    ///
    /// This sends the `IRCv3` [`CHATHISTORY TARGETS`] query.
    /// Pass [`crate::time::now`] as `until` to include conversations through the current time.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::History`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It can also return [`Error::InvalidInput`],
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    ///
    /// [`CHATHISTORY TARGETS`]: https://ircv3.net/specs/extensions/chathistory#targets
    pub fn recent_conversations(
        &self,
        since: SystemTime,
        until: SystemTime,
    ) -> crate::RequestFuture<Vec<RecentConversation>> {
        let client = self.clone();
        crate::RequestFuture::new(async move {
            client
                .history_response(HistoryQuery::__targets(since, until, 1_000))
                .await?
                .history_targets
                .ok_or_else(Error::connection_lost)
                .map(|targets| {
                    targets
                        .into_iter()
                        .map(|(conversation, latest_server_time)| RecentConversation {
                            conversation: crate::message_ref::stamp_conversation(
                                conversation,
                                client.origin(),
                            ),
                            latest_server_time,
                        })
                        .collect()
                })
        })
    }

    /// Marks everything seen in a conversation as read using its newest server timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::ReadMarkers`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. If no timestamped line has been seen,
    /// awaiting returns [`crate::InvalidInput::NoServerTime`]. A refusal returns [`Error::Server`] with
    /// [`crate::Refusal::Other`]. It returns [`Error::StateLimit`] before sending when a new target
    /// would exceed the configured read-marker bound. It can also return
    /// [`Error::TooManyPendingRequests`], [`Error::Timeout`], or [`Error::Disconnected`].
    pub fn mark_read(&self, target: impl crate::AsConversation) -> MarkReadRequest {
        MarkReadRequest {
            client: self.clone(),
            target: crate::AsConversation::__parts(&target).into_owned(),
            message: None,
        }
    }

    /// Queries the server read marker for a conversation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for [`crate::Feature::ReadMarkers`] with reason
    /// [`crate::UnsupportedReason::NotOffered`] or
    /// [`crate::UnsupportedReason::DisabledByClient`]. It can also return
    /// [`Error::InvalidInput`], [`Error::TooManyPendingRequests`], [`Error::Timeout`], or
    /// [`Error::Disconnected`]. A refusal returns [`Error::Server`] with its typed
    /// [`crate::Refusal`].
    pub fn fetch_read_marker(
        &self,
        target: impl crate::AsConversation,
    ) -> crate::RequestFuture<Option<SystemTime>> {
        let client = self.clone();
        let target = crate::AsConversation::__parts(&target).into_owned();
        crate::RequestFuture::new(async move {
            let target = client.resolve(
                crate::target::Role::Conversation("fetch_read_marker: conversation"),
                target,
            )?;
            let (result, receiver) = oneshot::channel();
            client
                .commands
                .send(Command::ReadMarker {
                    target: target.clone(),
                    result,
                })
                .await
                .map_err(|_| Error::connection_lost())?;
            let response = receiver.await.map_err(|_| Error::connection_lost())??;
            crate::requests::empty(&response)?;
            Ok(read_marker_time(client.state().read_marker(&target)))
        })
    }
}

fn read_marker_time(marker: Option<crate::ReadMarker>) -> Option<SystemTime> {
    match marker {
        Some(crate::ReadMarker::ReadUntil(time)) => Some(time),
        Some(crate::ReadMarker::Unset) | None => None,
    }
}

#[cfg(all(test, not(browser)))]
mod tests {
    use crate::driver::test_support::*;
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use papo_core::{
        Config, Connection, Event as CoreEvent, HistoryLine, HistoryQuery, MsgRef, Sasl, Session,
        sts::ConnectionSecurity,
    };
    use papo_proto::{Casemapping, Line, LineRef};

    use crate::{Event as PageEvent, MarkReadOutcome};

    use super::{
        HistoryPage, HistoryReason, HistoryReference, RecentConversation, history_item, reaction,
    };

    fn name_rules(state: &crate::State) -> crate::NameRules<'_> {
        state.name_rules().expect("server advertised CASEMAPPING")
    }

    #[test]
    fn mark_read_request_debug_is_opaque() {
        let secret = "unique-read-marker-target-8462";
        let request = super::MarkReadRequest {
            client: crate::Client::disconnected_for_test(),
            target: crate::AsConversation::__parts(&secret).into_owned(),
            message: None,
        };

        assert_eq!(format!("{request:?}"), "MarkReadRequest { .. }");
        assert!(!format!("{request:?}").contains(secret));
    }

    fn core_item(message: &str) -> HistoryLine {
        HistoryLine {
            line: LineRef::parse(message).unwrap().to_owned(),
            context: false,
            authorship: papo_core::Authorship::Unknown,
            subject_authorship: papo_core::Authorship::Unknown,
        }
    }

    fn historical_message(line: Line, received_at: SystemTime) -> crate::Message {
        let PageEvent::Message(message) = crate::event_impl::historical_message_event(
            line,
            crate::Authorship::Unknown,
            None,
            &crate::Client::disconnected_for_test(),
            received_at,
        ) else {
            panic!("expected a historical message")
        };
        message
    }

    #[test]
    fn live_and_history_build_the_same_actor_from_one_line() {
        let raw = "@account=alice;bot;draft/oper=network-admin;msgid=RfcBTzyxY120019 :alice!~a@staff.example PART #papo :Deploying release";
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_line(now, b":papo!~p@client.example JOIN #papo");
        session.handle_line(
            now,
            b"@account=alice :alice!~a@staff.example JOIN #papo alice :Alice Example",
        );
        while session.poll_event().is_some() {}
        session.handle_line(now, raw.as_bytes());
        let live = crate::event_impl::from_core(
            session.poll_event().expect("live part event"),
            session.isupport().casemapping(),
            crate::Origin::NONE,
            true,
            true,
            UNIX_EPOCH,
        );
        let crate::event_impl::CoreEvent::Forward(PageEvent::Parted(live)) = live else {
            panic!("expected live part")
        };

        let historical = history_item(
            papo_core::HistoryItem::Parted(HistoryLine {
                line: LineRef::parse(raw).unwrap().to_owned(),
                context: false,
                authorship: papo_core::Authorship::Other,
                subject_authorship: papo_core::Authorship::Unknown,
            }),
            None,
            &crate::Client::disconnected_for_test(),
            UNIX_EPOCH,
        );
        let PageEvent::Parted(historical) = historical else {
            panic!("expected historical part")
        };

        assert_eq!(live.actor, historical.actor);
    }

    #[test]
    fn converts_ids_timestamps_and_message_events() {
        assert!(
            matches!(HistoryReference::from(crate::MessageId::from("2Zl7JeYG6I8fG9q1lJ55BQ")).value, MsgRef::Msgid(id) if id == "2Zl7JeYG6I8fG9q1lJ55BQ")
        );
        assert!(
            matches!(HistoryReference::from(UNIX_EPOCH).value, MsgRef::Timestamp(time) if time == UNIX_EPOCH)
        );
        let oldest = historical_message(
            Line::privmsg("#papo", "earlier")
                .with_source("emily!~emily@localhost")
                .with_tag("msgid", "RfcBTzyxY120004"),
            UNIX_EPOCH,
        );
        let newest = historical_message(
            Line::privmsg("#papo", "later")
                .with_source("emily!~emily@localhost")
                .with_tag("msgid", "RfcBTzyxY120009"),
            UNIX_EPOCH,
        );
        let origin = crate::Origin::__new(73);
        let page = HistoryPage {
            conversation: crate::Conversation::Channel(
                origin.__channel(Casemapping::default(), "#papo"),
            ),
            items: vec![Event::Message(oldest), Event::Message(newest)],
            reason: HistoryReason::Requested,
            end: false,
        };
        let cursor = page.cursor().unwrap();
        assert_eq!(cursor.origin, Some(origin));
        assert!(matches!(cursor.value, MsgRef::Msgid(id)
            if id == "RfcBTzyxY120004"));
        assert!(matches!(cursor.after, Some(MsgRef::Msgid(id))
            if id == "RfcBTzyxY120009"));
    }

    #[test]
    fn incomplete_history_item_stays_out_of_complete_chat_messages() {
        let page = HistoryPage {
            conversation: crate::Conversation::Channel(Casemapping::default().channel("#papo")),
            items: vec![history_item(
                papo_core::HistoryItem::Incomplete(core_item(
                    ":alice!~a@cloak.example PRIVMSG #papo :unfinished deployment",
                )),
                None,
                &crate::Client::disconnected_for_test(),
                UNIX_EPOCH,
            )],
            reason: HistoryReason::Replay,
            end: false,
        };

        assert!(matches!(
            page.items.as_slice(),
            [PageEvent::Incomplete(crate::Incomplete::Message(message))]
                if message.text == "unfinished deployment"
        ));
        assert_eq!(page.chat_messages().count(), 0);
    }

    #[test]
    fn verified_private_history_carries_an_account_pin() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session.handle_connected_with(
            now,
            Connection {
                host: "irc.example".into(),
                security: ConnectionSecurity::Secure,
                authenticated_server_name: Some("irc.example".into()),
            },
        );
        session.handle_line(
            now,
            b":irc.example 005 papo CASEMAPPING=rfc1459 :are supported",
        );
        let (client, _) = client_with_commands(&session);
        let conversation =
            crate::Conversation::Private(client.state().name_rules().unwrap().nick("alice"));
        let event = history_item(
            papo_core::HistoryItem::Message(core_item(
                "@account=alice;time=2026-10-19T14:31:11.456Z :alice!~alice@cloak.example PRIVMSG papo :looks good",
            )),
            Some(&conversation),
            &client,
            UNIX_EPOCH,
        );
        let PageEvent::Message(message) = event else {
            panic!("expected historical private message")
        };

        assert_eq!(
            message
                .actor
                .peer_key()
                .and_then(crate::PeerKey::as_verified_str),
            Some(
                "papo-peer:1;server=irc.example;nick-casemap=rfc1459;nick=alice;proof=account;account-casemap=rfc1459;account=alice"
            )
        );
    }

    #[test]
    fn history_queries_reject_a_cursor_from_another_client() {
        let (first, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(Casemapping::default())
            .build();
        let (second, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(Casemapping::default())
            .build();
        let cursor = HistoryReference::new(
            MsgRef::Msgid("RfcBTzyxY120004".into()),
            Some(first.origin()),
        );

        assert!(matches!(
            second.history("#papo").after(cursor.clone()).query,
            Err(Error::InvalidInput(crate::InvalidInput::ForeignReference))
        ));
        assert!(matches!(
            second.history("#papo").latest_after(cursor).query,
            Err(Error::InvalidInput(crate::InvalidInput::ForeignReference))
        ));
    }

    #[test]
    fn history_conversations_are_bound_to_their_client() {
        let (first, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(Casemapping::default())
            .build();
        let (second, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(Casemapping::default())
            .build();
        for conversation in [
            crate::Conversation::Channel(
                first
                    .state()
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .channel("#papo"),
            ),
            crate::Conversation::Private(
                first
                    .state()
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .nick("emily"),
            ),
        ] {
            assert!(first.history(conversation.clone()).latest().query.is_ok());
            assert!(matches!(
                second.history(conversation).latest().query,
                Err(Error::InvalidInput(crate::InvalidInput::ForeignReference))
            ));
        }
    }

    #[test]
    fn history_results_report_their_origin_and_cannot_cross_clients() {
        let (first, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(Casemapping::default())
            .build();
        let (second, _, _) = crate::Client::builder("ircs://irc.example.net", "papo-client")
            .casemapping(Casemapping::default())
            .build();
        let page = HistoryPage {
            conversation: crate::Conversation::Channel(
                first
                    .state()
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .channel("#papo"),
            ),
            items: Vec::new(),
            reason: HistoryReason::Requested,
            end: false,
        };
        let recent = RecentConversation {
            conversation: crate::Conversation::Private(
                first
                    .state()
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .nick("emily"),
            ),
            latest_server_time: UNIX_EPOCH,
        };

        assert_eq!(page.origin(), first.origin());
        assert_eq!(recent.origin(), first.origin());
        for conversation in [page.conversation, recent.conversation] {
            assert!(matches!(
                second.history(conversation).latest().query,
                Err(Error::InvalidInput(crate::InvalidInput::ForeignReference))
            ));
        }
    }

    #[test]
    fn message_without_msgid_uses_its_time_as_reference() {
        let event = historical_message(
            Line::privmsg("#papo", "soju backlog")
                .with_source("emily!~emily@localhost")
                .with_tag("time", "1970-01-01T00:00:00.000Z"),
            UNIX_EPOCH,
        );

        let page = HistoryPage {
            conversation: event.conversation.clone(),
            items: vec![Event::Message(event)],
            reason: HistoryReason::Requested,
            end: false,
        };
        assert_eq!(page.cursor(), Some(HistoryReference::from(UNIX_EPOCH)));
    }

    #[test]
    fn page_cursors_never_use_local_receipt_time() {
        let received_at = UNIX_EPOCH + Duration::from_secs(1_792_520_531);
        let event = historical_message(
            Line::privmsg("#papo", "backlog without server metadata")
                .with_source("emily!~emily@cloak.example"),
            received_at,
        );
        let page = HistoryPage {
            conversation: event.conversation.clone(),
            items: vec![Event::Message(event)],
            reason: HistoryReason::Replay,
            end: false,
        };

        assert_eq!(page.cursor(), None);
    }

    #[test]
    fn preserves_proven_history_authorship_and_activity_context() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().allow_insecure_auth(true).sasl(
            Sasl::plain("papo", crate::Secret::new("correct horse battery staple")),
        ));
        session.handle_connected(now);
        for line in [
            "CAP * LS :sasl=PLAIN",
            "CAP * ACK :sasl",
            "AUTHENTICATE +",
            ":ergo.test 900 papo papo!~papo@localhost papo :You are now logged in as papo",
            ":ergo.test 903 papo :SASL authentication successful",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        while session.poll_event().is_some() {}
        for line in [
            ":ergo.test BATCH +history chathistory #papo",
            "@batch=history;account=papo;time=2026-10-19T14:31:08.123Z;msgid=RfcBTzyxY120004 :papo!~papo@localhost PRIVMSG #papo :deploy finished",
            "@batch=history;account=emily;time=2026-10-19T14:31:11.456Z;msgid=RfcBTzyxY120005 :emily!~emily@localhost PRIVMSG #papo :looks good",
            "@batch=history;time=2026-10-19T14:31:15.789Z;msgid=RfcBTzyxY120006 :papo!~papo@localhost PRIVMSG #papo :from an older server",
            "@batch=history;account=papo;time=2026-10-19T14:31:19.012Z;msgid=RfcBTzyxY120007 :papo-old!~papo@localhost NICK papo",
            "@batch=history;account=emily;time=2026-10-19T14:31:22.345Z;msgid=RfcBTzyxY120008 :emily!~emily@localhost QUIT :Connection reset by peer",
            ":ergo.test BATCH -history",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        let Some(CoreEvent::History {
            conversation,
            items,
            ..
        }) = session.poll_event()
        else {
            panic!("expected history event");
        };
        let client = crate::Client::disconnected_for_test();
        let items: Vec<_> = items
            .into_iter()
            .map(|item| history_item(item, Some(&conversation), &client, UNIX_EPOCH))
            .collect();

        assert!(matches!(&items[0], PageEvent::Message(message)
            if message.authorship == crate::Authorship::Own && message.is_own()));
        assert!(matches!(&items[1], PageEvent::Message(message)
            if message.authorship == crate::Authorship::Other && !message.is_own()));
        assert!(matches!(&items[2], PageEvent::Message(message)
            if message.authorship == crate::Authorship::Unknown && !message.is_own()));
        assert!(matches!(&items[3], PageEvent::NickChanged(changed)
            if changed.actor.nick() == "papo-old" && changed.new == "papo"
                && changed.channels == ["#papo"]
                && changed.authorship == crate::Authorship::Own));
        assert!(matches!(&items[4], PageEvent::Quit(quit)
            if quit.channels == ["#papo"]
                && quit.authorship == crate::Authorship::Other
                && matches!(quit.actor.account(), crate::Account::LoggedIn(account) if account == "emily")));
        let first_time = papo_proto::time::parse("2026-10-19T14:31:08.123Z").unwrap();
        assert!(matches!(&items[0], PageEvent::Message(message)
            if message.msgid.as_deref() == Some("RfcBTzyxY120004")
                && message.server_time == Some(first_time)
                && matches!(message.actor.account(), crate::Account::LoggedIn(account) if account == "papo")));
        assert!(matches!(&items[3], PageEvent::NickChanged(changed)
            if changed.msgid.as_deref() == Some("RfcBTzyxY120007")
                && matches!(changed.actor.account(), crate::Account::LoggedIn(account) if account == "papo")));
    }

    #[test]
    fn soju_history_without_msgids_pages_by_time() {
        let server_time = papo_proto::time::parse("2026-10-20T18:44:00.250Z").unwrap();
        let client = crate::Client::disconnected_for_test();
        let item = history_item(
            papo_core::HistoryItem::Message(core_item(
                "@time=2026-10-20T18:44:00.250Z :emily!~e@cloak.example PRIVMSG #papo :soju backlog",
            )),
            Some(&crate::Conversation::Channel(
                Casemapping::default().channel("#papo"),
            )),
            &client,
            UNIX_EPOCH,
        );
        let PageEvent::Message(message) = item else {
            panic!("expected a chat message");
        };
        assert_eq!(message.msgid, None);
        assert_eq!(message.server_time, Some(server_time));
        let page = HistoryPage {
            conversation: message.conversation.clone(),
            items: vec![Event::Message(message)],
            reason: HistoryReason::Requested,
            end: false,
        };
        let cursor = page.cursor().unwrap();
        assert!(matches!(&cursor.value, MsgRef::Timestamp(time) if *time == server_time));
        assert!(matches!(
            client.history("#papo").before(cursor).query,
            Ok(HistoryQuery::Before {
                before: MsgRef::Timestamp(time),
                ..
            }) if time == server_time
        ));
    }

    #[test]
    fn conversation_inputs_preserve_channel_and_private_addresses() {
        let client = crate::Client::disconnected_for_test();
        let state = client.state();
        for (conversation, expected) in [
            (
                crate::Conversation::Channel(
                    state
                        .name_rules()
                        .expect("server advertised CASEMAPPING")
                        .channel("#Papo[dev]"),
                ),
                "#Papo[dev]",
            ),
            (
                crate::Conversation::Private(
                    state
                        .name_rules()
                        .expect("server advertised CASEMAPPING")
                        .nick("Emily_"),
                ),
                "Emily_",
            ),
        ] {
            assert_eq!(
                state
                    .name_rules()
                    .expect("server advertised CASEMAPPING")
                    .conversation(&conversation.to_string())
                    .unwrap()
                    .to_string(),
                expected
            );
        }
    }

    #[test]
    fn maps_reactions_and_redactions_from_history() {
        let client = crate::Client::disconnected_for_test();
        let reacted = history_item(
            papo_core::HistoryItem::Reacted(core_item(
                "@account=emily;+draft/react=👍;+draft/reply=RfcBTzyxY120004 :emily!~e@localhost TAGMSG #papo",
            )),
            None,
            &client,
            UNIX_EPOCH,
        );
        assert!(matches!(
            &reacted,
            PageEvent::Reacted(event)
                if event.actor.nick() == "emily"
                    && matches!(&event.conversation, crate::Conversation::Channel(channel) if channel == "#papo")
                    && event.status.is_none()
                    && event.reacts_to == "RfcBTzyxY120004"
                    && event.reaction == "👍"
                    && matches!(event.actor.account(), crate::Account::LoggedIn(account) if account == "emily")
                    && event.action == crate::event::ReactionAction::Added
        ));
        let PageEvent::Reacted(event) = reacted else {
            unreachable!()
        };
        assert!(
            crate::Target::__parts(&event.return_address())
                .channel
                .is_some()
        );

        let removal = history_item(
            papo_core::HistoryItem::Redacted(core_item(
                "@account=alice :alice!~a@localhost REDACT #papo RfcBTzyxY120004 :",
            )),
            None,
            &client,
            UNIX_EPOCH,
        );
        assert!(matches!(
            &removal,
            PageEvent::Redacted(event)
                if matches!(&event.conversation, crate::Conversation::Channel(channel) if channel == "#papo")
                    && event.status.is_none()
                    && event.redacts == "RfcBTzyxY120004"
                    && matches!(event.actor.user().unwrap().account(), crate::Account::LoggedIn(account) if account == "alice")
                    && event.reason.is_none()
        ));
        let PageEvent::Redacted(event) = removal else {
            unreachable!()
        };
        assert!(
            crate::Target::__parts(&event.return_address())
                .channel
                .is_some()
        );
    }

    #[test]
    fn replayed_sources_capture_the_active_casemapping() {
        let line = LineRef::parse(
            "@+draft/react=👍;+draft/reply=RfcBTzyxY120004 :Alice[bot]!~a@cloak.example TAGMSG #papo",
        )
        .unwrap();
        let own_nick = Casemapping::Ascii.nick("papo");
        let Some(PageEvent::Reacted(event)) = reaction(
            &line,
            crate::Authorship::Unknown,
            Casemapping::Ascii,
            own_nick.origin(),
            &own_nick,
            &papo_proto::Isupport::default(),
            None,
            true,
            false,
            UNIX_EPOCH,
            None,
        ) else {
            panic!("reaction should be typed");
        };
        let nick = event.actor.nick();

        assert_eq!(nick.casemapping(), Casemapping::Ascii);
        assert_eq!(event.actor.account(), &crate::Account::LoggedOut);
        assert_ne!(nick, "alice{bot}");
    }

    #[test]
    fn malformed_typed_history_item_remains_raw() {
        let item = history_item(
            papo_core::HistoryItem::Redacted(core_item(":irc.example REDACT #papo")),
            None,
            &crate::Client::disconnected_for_test(),
            UNIX_EPOCH,
        );
        assert!(matches!(item, PageEvent::Other(line) if line.command() == "REDACT"));
    }

    #[test]
    fn ctcp_queries_in_history_are_typed_events() {
        let item = history_item(
            papo_core::HistoryItem::Message(core_item(
                "@msgid=RfcBTzyxY120009 :emily!~e@localhost PRIVMSG papo :\u{1}VERSION\u{1}",
            )),
            Some(&crate::Conversation::Private(
                Casemapping::default().nick("emily"),
            )),
            &crate::Client::disconnected_for_test(),
            UNIX_EPOCH,
        );

        assert!(matches!(item, Event::CtcpQuery(ctcp)
            if ctcp.command == "VERSION"
                && ctcp.msgid.as_deref() == Some("RfcBTzyxY120009")));
    }

    #[test]
    fn maps_replayed_channel_activity() {
        let client = crate::Client::disconnected_for_test();
        let joined = history_item(
            papo_core::HistoryItem::Joined(core_item(
                ":emily!~e@localhost JOIN #papo emily :Emily Smith",
            )),
            None,
            &client,
            UNIX_EPOCH,
        );
        assert!(matches!(
            joined,
            PageEvent::Joined(event)
                if event.channel == "#papo" && event.actor.nick() == "emily"
                    && event.channel.origin() == client.origin()
                    && event.actor.nick().origin() == client.origin()
                    && matches!(event.actor.account(), crate::Account::LoggedIn(account) if account == "emily" && account.origin() == client.origin())
                    && event.realname.as_deref() == Some("Emily Smith")
                    && event.authorship == crate::Authorship::Unknown
        ));

        let joined = history_item(
            papo_core::HistoryItem::Joined(core_item(":bob!~b@host JOIN #papo * :Bob")),
            None,
            &client,
            UNIX_EPOCH,
        );
        assert!(matches!(joined, PageEvent::Joined(event)
            if event.actor.account() == &crate::Account::LoggedOut
                && event.realname.as_deref() == Some("Bob")));

        let joined = history_item(
            papo_core::HistoryItem::Joined(core_item(":carol!~c@host JOIN #papo")),
            None,
            &client,
            UNIX_EPOCH,
        );
        assert!(matches!(joined, PageEvent::Joined(event)
            if event.actor.account() == &crate::Account::Unknown && event.realname.is_none()));
    }

    #[test]
    fn maps_each_replayed_membership_change() {
        let client = crate::Client::disconnected_for_test();
        let map = |item| history_item(item, None, &client, UNIX_EPOCH);

        let parted = map(papo_core::HistoryItem::Parted(core_item(
            ":emily!~e@localhost PART #papo :Leaving for lunch",
        )));
        assert!(matches!(
            parted,
            PageEvent::Parted(event)
                if event.channel == "#papo" && event.actor.nick() == "emily"
                    && event.channel.origin() == client.origin()
                    && event.actor.nick().origin() == client.origin()
                    && event.reason.as_deref() == Some("Leaving for lunch")
                    && event.authorship == crate::Authorship::Unknown
        ));

        let quit = map(papo_core::HistoryItem::Quit(core_item(
            ":emily!~e@localhost QUIT :Connection reset by peer",
        )));
        assert!(matches!(
            quit,
            PageEvent::Quit(event)
                if event.actor.nick() == "emily"
                    && event.actor.nick().origin() == client.origin()
                    && event.reason.as_deref() == Some("Connection reset by peer")
                    && event.channels.is_empty()
        ));

        let kicked = map(papo_core::HistoryItem::Kicked(core_item(
            ":irc.example KICK #papo emily :Requested by alice",
        )));
        assert!(matches!(
            kicked,
            PageEvent::Kicked(event)
                if event.channel == "#papo" && event.subject == "emily"
                    && event.channel.origin() == client.origin()
                    && event.subject.origin() == client.origin()
                    && event.reason.as_deref() == Some("Requested by alice")
                    && matches!(event.actor, crate::Source::Server(ref name) if name == "irc.example")
                    && event.subject_authorship == crate::Authorship::Unknown
        ));

        let renamed = map(papo_core::HistoryItem::NickChanged(core_item(
            ":emily!~e@localhost NICK emily_",
        )));
        assert!(matches!(
            renamed,
            PageEvent::NickChanged(event)
                if event.actor.nick() == "emily" && event.new == "emily_"
                    && event.actor.nick().origin() == client.origin()
                    && event.new.origin() == client.origin()
                    && event.authorship == crate::Authorship::Unknown
        ));
    }

    #[test]
    fn empty_replayed_departure_reasons_are_absent() {
        let client = crate::Client::disconnected_for_test();
        let map = |item| history_item(item, None, &client, UNIX_EPOCH);

        let items = [
            papo_core::HistoryItem::Parted(core_item(":emily!~e@localhost PART #papo :")),
            papo_core::HistoryItem::Quit(core_item(":emily!~e@localhost QUIT :")),
            papo_core::HistoryItem::Kicked(core_item(":alice!~a@localhost KICK #papo emily :")),
        ];
        for item in items {
            match map(item) {
                PageEvent::Parted(event) => assert_eq!(event.reason, None),
                PageEvent::Quit(event) => assert_eq!(event.reason, None),
                PageEvent::Kicked(event) => assert_eq!(event.reason, None),
                _ => panic!("expected a departure event"),
            }
        }
    }

    #[test]
    fn maps_replayed_topic_modes_and_unreaction() {
        let client = crate::Client::disconnected_for_test();
        let map = |item| history_item(item, None, &client, UNIX_EPOCH);

        let topic = map(papo_core::HistoryItem::TopicChanged(core_item(
            ":emily!~e@localhost TOPIC #papo :Rust 1.90 is out",
        )));
        assert!(matches!(
            topic,
            PageEvent::TopicChanged(event)
                if event.channel == "#papo"
                    && event.topic.as_deref() == Some("Rust 1.90 is out")
                    && event.actor.nick().is_some_and(|nick| nick == "emily")
        ));

        let mode = map(papo_core::HistoryItem::ModeChanged(core_item(
            ":emily!~e@localhost MODE #papo +im",
        )));
        assert!(matches!(
            mode,
            PageEvent::ModeChanged(event)
                if matches!(event.subject, crate::Subject::Channel(ref channel) if channel == "#papo")
                    && event.actor.nick().is_some_and(|nick| nick == "emily")
                    && event.changes.len() == 2
                    && event
                        .changes
                        .iter()
                        .all(|change| change.action == crate::ModeAction::Add)
                    && event.changes[0].mode == 'i' && event.changes[1].mode == 'm'
        ));

        let unreacted = map(papo_core::HistoryItem::Reacted(core_item(
            "@+draft/unreact=👍;+reply=RfcBTzyxY120004 :emily!~e@localhost TAGMSG #papo",
        )));
        assert!(matches!(
            unreacted,
            PageEvent::Reacted(event)
                if event.reaction == "👍"
                    && event.action == crate::event::ReactionAction::Removed
        ));
    }

    #[test]
    fn history_starting_points_default_to_100_and_accept_an_override() {
        let client = crate::Client::disconnected_for_test();
        let latest = client.history("#papo").latest();
        assert!(matches!(
            latest.query,
            Ok(HistoryQuery::Latest { limit: 100, .. })
        ));
        let message_id = crate::MessageId::from("RfcBTzyxY120000");
        let before = client.history("#papo").before(message_id.clone());
        assert!(matches!(
            before.query,
            Ok(HistoryQuery::Before { limit: 100, .. })
        ));
        let after = client
            .history("#papo")
            .after(UNIX_EPOCH + Duration::from_secs(42));
        assert!(matches!(
            after.query,
            Ok(HistoryQuery::After { limit: 100, .. })
        ));
        let latest_after = client
            .history("#papo")
            .latest_after(crate::MessageId::from("RfcBTzyxY120000"));
        assert!(matches!(
            latest_after.query,
            Ok(HistoryQuery::Latest {
                after: Some(MsgRef::Msgid(_)),
                limit: 100,
                ..
            })
        ));
        let around = client.history("#papo").around(&message_id);
        assert!(matches!(
            around.query,
            Ok(HistoryQuery::Around { limit: 100, .. })
        ));
        let between = client.history("#papo").between(
            crate::MessageId::from("RfcBTzyxY119998"),
            crate::MessageId::from("RfcBTzyxY120003"),
        );
        assert!(matches!(
            between.query,
            Ok(HistoryQuery::Between { limit: 100, .. })
        ));
        let override_limit = client.history("#papo").latest().limit(51);
        assert!(matches!(
            override_limit.query,
            Ok(HistoryQuery::Latest { limit: 51, .. })
        ));
    }

    #[test]
    fn page_cursor_selects_the_requested_edge() {
        let client = crate::Client::disconnected_for_test();
        let cursor = HistoryReference {
            value: MsgRef::Msgid("RfcBTzyxY120004".into()),
            after: Some(MsgRef::Msgid("RfcBTzyxY120009".into())),
            origin: None,
        };

        assert!(matches!(
            client.history("#papo").before(cursor.clone()).query,
            Ok(HistoryQuery::Before { before: MsgRef::Msgid(id), .. })
                if id == "RfcBTzyxY120004"
        ));
        assert!(matches!(
            client.history("#papo").after(cursor.clone()).query,
            Ok(HistoryQuery::After { after: MsgRef::Msgid(id), .. })
                if id == "RfcBTzyxY120009"
        ));
        assert!(matches!(
            client.history("#papo").latest_after(cursor).query,
            Ok(HistoryQuery::Latest { after: Some(MsgRef::Msgid(id)), .. })
                if id == "RfcBTzyxY120009"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn history_page_revalidates_target_against_current_server_rules() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let request = client.history("#papo-history").latest().limit(10);
        let task = tokio::spawn(driver);
        welcome_burst(&mut server).await;
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
        server_line(&mut server, b":ergo.test 005 papo-driver CHANTYPES=! NICKLEN=9 CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n:ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::Connected(_))
        ));

        let result = within(async { request.await }).await;
        assert!(
            matches!(
                result,
                Err(Error::InvalidInput(crate::InvalidInput::InvalidName { ref name, .. }))
                if name == "#papo-history"
            ),
            "{result:?}"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(1), server.read_u8())
                .await
                .is_err()
        );
        abort_task(task).await;
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
        welcome_burst(&mut server).await;
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
        server_line(&mut server, b":ergo.test 005 papo-driver CHANMODES=Ibe,k,fl,CEMRUimnstu PREFIX=(qaohv)~&@%+ CHATHISTORY=100 MSGREFTYPES=msgid,timestamp STATUSMSG=@+ :supported\r\n:ergo.test 001 papo-driver :Welcome\r\n:ergo.test 422 papo-driver :MOTD File is missing\r\n").await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::Connected(_))
        ));
        assert!(matches!(
            client.history("#papo invalid").latest().await,
            Err(Error::InvalidInput(crate::InvalidInput::InvalidName { name, .. }))
                if name == "#papo invalid"
        ));
        assert!(matches!(
            client.history("@#papo").latest().await,
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.history("*").latest().await,
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.history("$opers").latest().await,
            Err(Error::InvalidInput(_))
        ));
        let conversation =
            crate::Conversation::Channel(name_rules(&client.state()).channel("#papo"));
        let request = tokio::spawn(async move { client.history(conversation).latest().await });
        client_line(&mut server, b"@label=0 CHATHISTORY LATEST #papo * 100\r\n").await;
        server_line(&mut server, b"@label=0 :ergo.test BATCH +reply labeled-response\r\n@batch=reply;draft/chathistory-end :ergo.test BATCH +history chathistory #papo\r\n@batch=history;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120004 :emily!~e@localhost PRIVMSG #papo :earlier\r\n@batch=history;+draft/react=eyes;+reply=RfcBTzyxY120004 :emily!~e@localhost TAGMSG #papo\r\n@batch=history :emily!~e@localhost JOIN #papo emily :Emily Smith\r\n@batch=history :ChanServ!service@services MODE #papo +o alice\r\n@batch=history :OperServ!service@services MODE #papo +b *!*@spammer.example\r\n@batch=history :emily!~e@localhost TAGMSG #papo\r\n@batch=reply :ergo.test BATCH -history\r\n:ergo.test BATCH -reply\r\n").await;
        let page = within(request).await.unwrap().unwrap();
        assert_eq!(page.reason, HistoryReason::Requested);
        assert!(page.end);
        assert_eq!(page.items.len(), 6);
        assert!(matches!(page.items[1], Event::Reacted(_)));
        assert!(matches!(page.items[2], Event::Joined(_)));
        let Event::ModeChanged(prefix_mode) = &page.items[3] else {
            panic!("expected replayed prefix mode")
        };
        assert!(matches!(
            prefix_mode.changes.as_slice(),
            [crate::ModeChange {
                mode: 'o',
                action: crate::ModeAction::Add,
                arg: Some(crate::ModeArg::Member { nick, .. }),
                ..
            }] if nick == "alice"
        ));
        let Event::ModeChanged(list_mode) = &page.items[4] else {
            panic!("expected replayed list mode")
        };
        assert!(matches!(
            list_mode.changes.as_slice(),
            [crate::ModeChange {
                mode: 'b',
                action: crate::ModeAction::Add,
                arg: Some(crate::ModeArg::Mask(mask)),
                ..
            }] if mask == "*!*@spammer.example"
        ));
        assert!(matches!(&page.items[5], Event::TagMsg(_)));
        let message = page.chat_messages().next().unwrap();
        assert_eq!(message.text, "earlier");
        assert!(
            tokio::time::timeout(Duration::from_millis(1), events.next())
                .await
                .is_err()
        );
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn private_history_uses_the_requested_conversation_after_a_nick_change() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
            "message-tags",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_connect_caps(
            &mut server,
            &mut events,
            "draft/chathistory labeled-response batch server-time message-tags",
            "draft/chathistory labeled-response batch server-time message-tags",
        )
        .await;
        server_line(&mut server, b":ergo.test 005 papo-driver CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :supported\r\n").await;
        server_line(
            &mut server,
            b":papo-driver!p@localhost JOIN #review\r\n:alice!~a@host JOIN #review\r\n",
        )
        .await;
        for _ in 0..2 {
            assert!(matches!(
                next_event(&mut events).await,
                Some(Event::Joined(_))
            ));
        }

        let request = tokio::spawn({
            let client = client.clone();
            async move { client.history("alice").latest().await }
        });
        client_line(&mut server, b"@label=0 CHATHISTORY LATEST alice * 100\r\n").await;
        server_line(&mut server, b"@label=0 :soju.example BATCH +reply labeled-response\r\n@batch=reply;draft/chathistory-end :soju.example BATCH +history chathistory alice\r\n@batch=history;time=2026-10-15T12:00:00.000Z;msgid=RfcBTzyxY120004 :alice!~a@host PRIVMSG papo_away :can you send me the invoice?\r\n@batch=reply :soju.example BATCH -history\r\n:soju.example BATCH -reply\r\n").await;

        let page = within(request).await.unwrap().unwrap();
        let message = page.chat_messages().next().unwrap();
        assert!(matches!(
            message.conversation,
            crate::Conversation::Private(ref nick) if nick == "alice"
        ));
        assert_eq!(message.return_address(), "alice");
        let reaction = tokio::spawn({
            let client = client.clone();
            let message = message.clone();
            async move { client.react(&message, "👍").await }
        });
        client_line(
            &mut server,
            b"@+reply=RfcBTzyxY120004;+draft/react=\xF0\x9F\x91\x8D;label=1 TAGMSG alice\r\n",
        )
        .await;
        server_line(&mut server, b"@label=1 :soju.example ACK\r\n").await;
        within(reaction).await.unwrap().unwrap();

        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn latest_after_history_reaches_the_wire_for_time_and_message_id() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_connect_history(&mut server, &mut events).await;

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("#papo")
                    .latest_after(UNIX_EPOCH + Duration::from_secs(42))
                    .limit(54)
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=0 CHATHISTORY LATEST #papo timestamp=1970-01-01T00:00:42.000Z 54\r\n",
        )
        .await;
        history_response(&mut server, 0).await;
        within(request).await.unwrap().unwrap();

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("alice")
                    .latest_after(crate::MessageId::from("RfcBTzyxY120000"))
                    .limit(55)
                    .await
            }
        });
        client_line(
            &mut server,
            b"@label=1 CHATHISTORY LATEST alice msgid=RfcBTzyxY120000 55\r\n",
        )
        .await;
        history_response(&mut server, 1).await;
        within(request).await.unwrap().unwrap();
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
        complete_connect_history(&mut server, &mut events).await;

        let request = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .history("#papo")
                    .before(crate::MessageId::from("RfcBTzyxY119999"))
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

        let message_id = before
            .chat_messages()
            .next()
            .unwrap()
            .msgid
            .clone()
            .unwrap();
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.history("#papo").around(&message_id).limit(52).await }
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
                .between(
                    crate::MessageId::from("RfcBTzyxY119998"),
                    crate::MessageId::from("RfcBTzyxY120003"),
                )
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
        complete_connect(&mut server, &mut events).await;
        assert!(matches!(
            within(async { client.history("#papo").latest().limit(10).await }).await,
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
    async fn recent_conversations_are_typed_and_time_bounded() {
        let config = config().wanted_caps([
            "draft/chathistory",
            "labeled-response",
            "batch",
            "server-time",
        ]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_connect_history(&mut server, &mut events).await;
        let channel = client
            .state()
            .name_rules()
            .expect("server advertised CASEMAPPING")
            .channel("#papo");
        let request = tokio::spawn(async move {
            client
                .recent_conversations(
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
        assert_eq!(
            targets[0].conversation,
            crate::Conversation::Channel(channel)
        );
        assert_eq!(
            targets[0].latest_server_time,
            UNIX_EPOCH + Duration::from_hours(497_796)
        );
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    #[allow(clippy::too_many_lines)]
    async fn mark_read_ignores_local_clock_and_reports_typed_outcomes() {
        let config = config().wanted_caps(["draft/read-marker", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_connect_read_marker(&mut server, &mut events).await;
        assert!(matches!(
            client.mark_read("@#papo").await,
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.mark_read("*").await,
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.mark_read("$opers").await,
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(client.state().read_marker("#papo"), None);
        let channel = crate::Conversation::Channel(
            client
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .channel("#papo"),
        );
        server_line(
            &mut server,
            b"@time=2025-10-15T11:55:42.123Z;msgid=Newest :alice!a@host PRIVMSG #papo :seen\r\n",
        )
        .await;
        let message = loop {
            if let Some(Event::Message(message)) = next_event(&mut events).await {
                break message.return_address();
            }
        };
        let request_client = client.clone();
        let request = tokio::spawn(async move { request_client.mark_read(channel).await });
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
            MarkReadOutcome::Advanced(UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000))
        );
        assert!(
            matches!(next_event(&mut events).await, Some(Event::ReadMarkerChanged(marker)) if matches!(&marker.conversation, crate::Conversation::Channel(channel) if channel == "#papo"))
        );

        let request_client = client.clone();
        let unchanged = tokio::spawn(async move { request_client.mark_read("#PAPO").await });
        client_line(
            &mut server,
            b"@label=1 MARKREAD #PAPO timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test MARKREAD #PAPO timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        assert_eq!(
            within(unchanged).await.unwrap().unwrap(),
            MarkReadOutcome::Unchanged {
                stored: UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000),
            }
        );

        server_line(
            &mut server,
            b":ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:43.000Z\r\n",
        )
        .await;
        assert!(matches!(
            next_event(&mut events).await,
            Some(Event::ReadMarkerChanged(_))
        ));
        let request_client = client.clone();
        let behind =
            tokio::spawn(async move { request_client.mark_read("#papo").up_to(&message).await });
        client_line(
            &mut server,
            b"@label=2 MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:43.000Z\r\n",
        )
        .await;
        assert_eq!(
            within(behind).await.unwrap().unwrap(),
            MarkReadOutcome::BehindLatest {
                stored: UNIX_EPOCH + Duration::from_secs(1_760_529_343),
                latest: UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000),
            }
        );
        server_line(
            &mut server,
            b"@time=2025-10-15T11:55:44.000Z;msgid=Foreign :bob!b@host PRIVMSG #other :elsewhere\r\n",
        )
        .await;
        let foreign = loop {
            if let Some(Event::Message(message)) = next_event(&mut events).await {
                break message.return_address();
            }
        };
        assert!(matches!(
            client.mark_read("#papo").up_to(&foreign).await,
            Err(Error::InvalidInput(crate::InvalidInput::WrongConversation))
        ));
        assert!(matches!(
            client.mark_read("#empty").await,
            Err(Error::InvalidInput(crate::InvalidInput::NoServerTime))
        ));
        server_line(
            &mut server,
            b"@time=2025-10-15T11:55:45.001Z;msgid=Boundary :carol!c@host PRIVMSG #boundary :reviewed\r\n",
        )
        .await;
        let boundary = loop {
            if let Some(Event::Message(message)) = next_event(&mut events).await {
                break message.return_address();
            }
        };
        let request_client = client.clone();
        let unconfirmed =
            tokio::spawn(
                async move { request_client.mark_read("#boundary").up_to(&boundary).await },
            );
        client_line(
            &mut server,
            b"@label=3 MARKREAD #boundary timestamp=2025-10-15T11:55:45.001Z\r\n",
        )
        .await;
        server_line(
            &mut server,
            b"@label=3 :ergo.test MARKREAD #boundary timestamp=2025-10-15T11:55:45.000Z\r\n",
        )
        .await;
        assert!(matches!(
            within(unconfirmed).await.unwrap(),
            Err(Error::Unconfirmed)
        ));
        abort_task(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn read_marker_queries_update_and_return_cached_state() {
        let config = config().wanted_caps(["draft/read-marker", "labeled-response"]);
        let (mut server, client, mut events, driver) = scripted(config);
        let task = tokio::spawn(driver);
        complete_connect_read_marker(&mut server, &mut events).await;
        assert!(matches!(
            client.fetch_read_marker("@#papo").await,
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.fetch_read_marker("*").await,
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client.fetch_read_marker("$opers").await,
            Err(Error::InvalidInput(_))
        ));
        let channel = crate::Conversation::Channel(
            client
                .state()
                .name_rules()
                .expect("server advertised CASEMAPPING")
                .channel("#papo"),
        );
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.fetch_read_marker(channel).await }
        });
        client_line(&mut server, b"@label=0 MARKREAD #papo\r\n").await;
        server_line(
            &mut server,
            b"@label=0 :ergo.test MARKREAD #papo timestamp=*\r\n",
        )
        .await;
        assert_eq!(within(request).await.unwrap().unwrap(), None);
        assert_eq!(
            client.state().read_marker("#PAPO"),
            Some(crate::ReadMarker::Unset)
        );

        let request = tokio::spawn({
            let client = client.clone();
            async move { client.fetch_read_marker("#papo").await }
        });
        client_line(&mut server, b"@label=1 MARKREAD #papo\r\n").await;
        server_line(
            &mut server,
            b"@label=1 :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        let expected = UNIX_EPOCH + Duration::new(1_760_529_342, 123_000_000);
        assert_eq!(within(request).await.unwrap().unwrap(), Some(expected));
        assert_eq!(
            client.state().read_marker("#PAPO"),
            Some(crate::ReadMarker::ReadUntil(expected))
        );
        let foreign = crate::Origin::__new(91).__channel(client.state().casemapping(), "#papo");
        assert_eq!(client.state().read_marker(foreign), None);
        assert_eq!(client.state().read_marker("@#papo"), None);

        let request = tokio::spawn(async move { client.fetch_read_marker("#papo").await });
        client_line(&mut server, b"@label=2 MARKREAD #papo\r\n").await;
        server_line(
            &mut server,
            b"@label=2 :ergo.test MARKREAD #papo timestamp=2025-10-15T11:55:42.123Z\r\n",
        )
        .await;
        assert_eq!(within(request).await.unwrap().unwrap(), Some(expected));
        abort_task(task).await;
    }
}
