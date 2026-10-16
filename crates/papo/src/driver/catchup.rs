use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::SystemTime,
};

use papo_core::{HistoryQuery, RequestId, Response, Session};
use papo_proto::Message;

use super::{EventSink, PostRegistration, now};
use crate::{Client, Event};

pub(super) struct Catchup {
    pub(super) target: String,
    pub(super) messages: Vec<Message>,
    pub(super) seen: HashSet<String>,
}

pub(super) fn advance_catchup(
    session: &mut Session,
    events: &EventSink,
    catchups: &mut BTreeMap<RequestId, Catchup>,
    mut catchup: Catchup,
    response: Response,
) -> Option<Event> {
    let page = response.history?;
    let next = extend_catchup(
        &mut catchup,
        page.messages.into_iter().map(|item| item.message),
    );
    if !page.end
        && catchup.messages.len() < 1_000
        && let Some(next) = next
    {
        let query = HistoryQuery::After {
            target: catchup.target.clone(),
            after: papo_core::MsgRef::Msgid(next),
            limit: (1_000 - catchup.messages.len()).min(100),
        };
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(id, catchup);
            return None;
        }
    }
    let commands = events.commands.upgrade()?;
    let client = Client {
        commands,
        connected: Arc::clone(&events.connected),
        state: Arc::clone(&events.state),
    };
    let messages = catchup
        .messages
        .into_iter()
        .filter_map(|message| {
            crate::MessageEvent::from_history(message, client.clone(), SystemTime::now()).ok()
        })
        .collect();
    Some(Event::History {
        target: catchup.target,
        messages,
        end: page.end,
    })
}

pub(super) fn extend_catchup(
    catchup: &mut Catchup,
    messages: impl IntoIterator<Item = Message>,
) -> Option<String> {
    let mut next = None;
    for message in messages {
        let msgid = message
            .as_ref()
            .tag("msgid")
            .map(std::borrow::Cow::into_owned);
        next.clone_from(&msgid);
        if msgid
            .as_ref()
            .is_none_or(|msgid| catchup.seen.insert(msgid.clone()))
        {
            catchup.messages.push(message);
        }
        if catchup.messages.len() == 1_000 {
            break;
        }
    }
    next
}

pub(super) fn begin_catchup(
    session: &mut Session,
    defaults: &PostRegistration,
    catchups: &mut BTreeMap<RequestId, Catchup>,
) {
    if !defaults.history_on_reconnect || !should_fetch_history(session) {
        return;
    }
    let seen = defaults
        .channels
        .iter()
        .filter_map(|channel| session.state().last_seen(channel).cloned())
        .collect::<Vec<_>>();
    for seen in seen {
        let query = HistoryQuery::After {
            target: seen.target.clone(),
            after: papo_core::MsgRef::Msgid(seen.msgid.clone()),
            limit: 100,
        };
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(
                id,
                Catchup {
                    target: seen.target,
                    messages: Vec::new(),
                    seen: HashSet::from([seen.msgid]),
                },
            );
        }
    }
}

pub(super) fn should_fetch_history(session: &Session) -> bool {
    !session.server_rejoins_channels()
}
