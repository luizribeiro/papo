use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use super::{EventSink, PostRegistration, now};
use crate::{Client, Event};
use papo_core::{HistoryQuery, RequestId, Response, Session};

pub(super) struct Catchup {
    pub(super) target: String,
    pub(super) items: Vec<papo_core::HistoryItem>,
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
    let next = extend_catchup(&mut catchup, page.items);
    if !page.end
        && catchup.items.len() < 1_000
        && let Some(next) = next
    {
        let query = HistoryQuery::After {
            target: catchup.target.clone(),
            after: papo_core::MsgRef::Msgid(next),
            limit: (1_000 - catchup.items.len()).min(100),
        };
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(id, catchup);
            return None;
        }
    }
    let commands = events.commands.upgrade()?;
    let client = Client {
        commands,
        state: Arc::clone(&events.state),
    };
    let items = catchup
        .items
        .into_iter()
        .map(|item| {
            crate::ext::history::history_item(
                item,
                &client,
                crate::rt::now_wall(),
                crate::Origin::History,
            )
        })
        .collect();
    Some(Event::History(Box::new(crate::HistoryPage {
        target: catchup.target.into(),
        items,
        end: page.end,
    })))
}

pub(super) fn extend_catchup(
    catchup: &mut Catchup,
    items: impl IntoIterator<Item = papo_core::HistoryItem>,
) -> Option<String> {
    let mut next = None;
    for item in items {
        let msgid = item
            .clone()
            .into_raw()
            .message
            .as_ref()
            .tag("msgid")
            .map(std::borrow::Cow::into_owned);
        next.clone_from(&msgid);
        if msgid
            .as_ref()
            .is_none_or(|msgid| catchup.seen.insert(msgid.clone()))
        {
            catchup.items.push(item);
        }
        if catchup.items.len() == 1_000 {
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
        .filter_map(|channel| session.state().last_seen(&channel.channel).cloned())
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
                    items: Vec::new(),
                    seen: HashSet::from([seen.msgid]),
                },
            );
        }
    }
}

pub(super) fn should_fetch_history(session: &Session) -> bool {
    !session.server_rejoins_channels()
}
