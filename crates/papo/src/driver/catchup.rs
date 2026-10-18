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
    let supports_timestamp = session.isupport().msgreftypes().contains(&"timestamp");
    for channel in &defaults.channels {
        let seen = session.state().last_seen(&channel.channel).cloned();
        let after = match &seen {
            Some(seen) => papo_core::MsgRef::Msgid(seen.msgid.clone()),
            None if supports_timestamp => {
                let Some(disconnected_at) = defaults.disconnected_at else {
                    continue;
                };
                papo_core::MsgRef::Timestamp(disconnected_at)
            }
            None => continue,
        };
        let query = HistoryQuery::After {
            target: channel.channel.clone(),
            after,
            limit: 100,
        };
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(
                id,
                Catchup {
                    target: channel.channel.clone(),
                    items: Vec::new(),
                    seen: seen.into_iter().map(|seen| seen.msgid).collect(),
                },
            );
        }
    }
}

pub(super) fn should_fetch_history(session: &Session) -> bool {
    #[cfg(not(feature = "unstable"))]
    let _ = session;
    #[cfg(feature = "unstable")]
    return !session.server_rejoins_channels();
    #[cfg(not(feature = "unstable"))]
    true
}
