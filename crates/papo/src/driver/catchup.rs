use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::SystemTime,
};

use super::{EventSink, PostConnect, now};
use crate::{Client, Event};
use papo_core::{HistoryQuery, RequestId, Response, Session};

pub(super) struct Catchup {
    pub(super) target: String,
    pub(super) items: Vec<papo_core::HistoryItem>,
    pub(super) seen: HashSet<String>,
}

#[derive(Clone, Debug)]
pub(super) struct CatchupCursor {
    msgid: Option<String>,
    time: SystemTime,
}

pub(super) fn remember_live_reaction<'a>(
    catchups: impl IntoIterator<Item = &'a mut Catchup>,
    reaction: &crate::event::Reacted,
) {
    let Some(msgid) = &reaction.msgid else {
        return;
    };
    for catchup in catchups {
        let matches = match &reaction.conversation {
            crate::Conversation::Channel(channel) => channel == catchup.target.as_str(),
            _ => false,
        };
        if matches {
            catchup.seen.insert(msgid.to_string());
        }
    }
}

impl CatchupCursor {
    fn reference(&self, supports_timestamp: bool) -> Option<papo_core::MsgRef> {
        self.msgid
            .clone()
            .map(papo_core::MsgRef::Msgid)
            .or_else(|| supports_timestamp.then_some(papo_core::MsgRef::Timestamp(self.time)))
    }
}

pub(super) fn advance_catchup(
    session: &mut Session,
    events: &EventSink,
    catchups: &mut BTreeMap<RequestId, Catchup>,
    defaults: &mut PostConnect,
    mut catchup: Catchup,
    response: Response,
) -> Option<Event> {
    let page = response.history?;
    let conversation = page.conversation;
    let page_end = page.end;
    let next = extend_catchup(&mut catchup, page.items);
    if !page_end
        && catchup.items.len() < 1_000
        && let Some(next) = next
    {
        let query = HistoryQuery::__after(
            catchup.target.clone(),
            papo_core::MsgRef::Msgid(next),
            (1_000 - catchup.items.len()).min(100),
        );
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(id, catchup);
            return None;
        }
    }
    let commands = events.commands.upgrade()?;
    if let Some(cursor) = newest_cursor(&catchup.items)
        && let Some(channel) = defaults.channels.iter_mut().find(|channel| {
            session.isupport().casemapping().channel(&channel.channel) == catchup.target.as_str()
        })
        && channel
            .catchup_cursor
            .as_ref()
            .is_none_or(|previous| previous.time <= cursor.time)
    {
        channel.catchup_cursor = Some(cursor);
    }
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
                Some(&conversation),
                &client,
                crate::rt::now_wall(),
            )
        })
        .collect();
    Some(Event::History(Box::new(crate::HistoryPage {
        conversation,
        items,
        end: page_end,
        reason: crate::HistoryReason::Catchup,
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
            .into_line()
            .line
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

fn newest_cursor(items: &[papo_core::HistoryItem]) -> Option<CatchupCursor> {
    items
        .iter()
        .filter_map(|item| {
            let raw = item.clone().into_line().line;
            let message = raw.as_ref();
            let time = message
                .tag("time")
                .and_then(|value| papo_proto::time::parse(&value))?;
            Some(CatchupCursor {
                msgid: message.tag("msgid").map(std::borrow::Cow::into_owned),
                time,
            })
        })
        .max_by_key(|cursor| cursor.time)
}

pub(super) fn begin_catchup(
    session: &mut Session,
    defaults: &PostConnect,
    catchups: &mut BTreeMap<RequestId, Catchup>,
) {
    if !defaults.history_on_reconnect || server_restores_channels(session) {
        return;
    }
    let supports_timestamp = session.isupport().msgreftypes().contains(&"timestamp");
    for channel in &defaults.channels {
        let seen = session.state().last_seen(&channel.channel).cloned();
        let cursor = channel.catchup_cursor.as_ref().filter(|cursor| {
            seen.as_ref()
                .is_none_or(|seen| cursor.time >= seen.server_time)
        });
        let after = if let Some(reference) = cursor
            .and_then(|cursor| cursor.reference(supports_timestamp))
            .or_else(|| {
                seen.as_ref()
                    .map(|seen| papo_core::MsgRef::Msgid(seen.msgid.clone()))
            }) {
            reference
        } else if supports_timestamp {
            let Some(disconnected_at) = defaults.disconnected_at else {
                continue;
            };
            papo_core::MsgRef::Timestamp(disconnected_at)
        } else {
            continue;
        };
        let query = HistoryQuery::__after(channel.channel.clone(), after, 100);
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(
                id,
                Catchup {
                    target: channel.channel.clone(),
                    items: Vec::new(),
                    seen: cursor
                        .and_then(|cursor| cursor.msgid.clone())
                        .or_else(|| seen.map(|seen| seen.msgid))
                        .into_iter()
                        .collect(),
                },
            );
        }
    }
}

#[cfg(feature = "unstable")]
pub(super) fn server_restores_channels(session: &Session) -> bool {
    session.server_rejoins_channels()
}

#[cfg(not(feature = "unstable"))]
pub(super) const fn server_restores_channels(_session: &Session) -> bool {
    false
}

#[cfg(all(test, not(browser)))]
mod tests {
    use std::{collections::BTreeMap, time::SystemTime};

    use papo_core::{Config, HistoryItem, HistoryLine, MsgRef, RateLimit, Session};
    use papo_proto::LineRef;

    fn registered_history_session() -> Session {
        let now = std::time::Instant::now();
        let mut session = Session::new(
            Config::default()
                .rate_limit(RateLimit::unlimited())
                .wanted_caps(["draft/chathistory", "labeled-response"]),
        );
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * LS :draft/chathistory labeled-response\r\n",
        );
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response\r\n",
        );
        while session.poll_transmit().is_some() {}
        session.handle_bytes(now, b":ergo.test 005 papo CHATHISTORY=100 MSGREFTYPES=msgid,timestamp :are supported by this server\r\n");
        session.handle_bytes(
            now,
            b":ergo.test 001 papo :Welcome to the ErgoTest IRC Network\r\n",
        );
        while session.poll_event().is_some() {}
        session
    }

    fn reconnect_defaults(history_on_reconnect: bool) -> super::super::PostConnect {
        super::super::PostConnect {
            bot: false,
            channels: vec!["#papo".into()],
            parted_while_disconnected: Vec::new(),
            history_on_reconnect,
            disconnected_at: Some(SystemTime::UNIX_EPOCH),
        }
    }

    #[test]
    fn gap_fill_deduplicates_a_live_reaction_by_msgid() {
        let mapping = papo_proto::Casemapping::default();
        let mut catchup = super::Catchup {
            target: "#papo".into(),
            items: Vec::new(),
            seen: std::collections::HashSet::new(),
        };
        let live = crate::event::Reacted {
            msgid: Some("RfcBTzyxY120010".into()),
            time: SystemTime::UNIX_EPOCH,
            server_time: None,
            account: Some(mapping.account("emily")),
            actor: crate::Source::from_proto(
                papo_proto::Source::__user("emily", Some("~e"), Some("cloak.example")),
                mapping,
            ),
            authorship: crate::Authorship::Other,
            conversation: crate::Conversation::Channel(mapping.channel("#papo")),
            status: None,
            message: "RfcBTzyxY120008".into(),
            reaction: "👍".into(),
            removed: false,
        };
        super::remember_live_reaction(std::iter::once(&mut catchup), &live);
        super::extend_catchup(
            &mut catchup,
            [
                "@time=2026-10-20T18:42:12.456Z;msgid=RfcBTzyxY120010;+draft/react=👍;+draft/reply=RfcBTzyxY120008 :emily!~e@cloak.example TAGMSG #papo",
                "@time=2026-10-20T18:42:13.789Z;msgid=RfcBTzyxY120011;+draft/react=🎉;+draft/reply=RfcBTzyxY120009 :alice!~a@cloak.example TAGMSG #papo",
            ]
            .map(|line| papo_core::HistoryItem::Reacted(history_line(line))),
        );

        assert_eq!(catchup.items.len(), 1);
        assert_eq!(
            catchup.items[0]
                .clone()
                .into_line()
                .line
                .as_ref()
                .tag("msgid")
                .as_deref(),
            Some("RfcBTzyxY120011")
        );
    }

    fn history_line(line: &str) -> papo_core::HistoryLine {
        papo_core::HistoryLine {
            line: LineRef::parse(line).unwrap().to_owned(),
            context: false,
            authorship: papo_core::Authorship::Unknown,
            subject_authorship: papo_core::Authorship::Unknown,
        }
    }

    #[test]
    fn activity_without_a_msgid_advances_the_timestamp_cursor() {
        let time = papo_proto::time::parse("2026-10-15T12:01:00.000Z").unwrap();
        let item = HistoryItem::Joined(HistoryLine {
            line: LineRef::parse("@time=2026-10-15T12:01:00.000Z :papo!papo@localhost JOIN #papo")
                .unwrap()
                .to_owned(),
            context: false,
            authorship: papo_core::Authorship::Own,
            subject_authorship: papo_core::Authorship::Unknown,
        });
        let cursor = super::newest_cursor(&[item]).unwrap();
        assert_eq!(cursor.reference(true), Some(MsgRef::Timestamp(time)));
        assert_eq!(cursor.reference(false), None);
    }

    #[test]
    fn reconnect_without_timestamp_references_skips_an_unknown_cursor() {
        let now = std::time::Instant::now();
        let mut session = Session::new(
            Config::default()
                .rate_limit(RateLimit::unlimited())
                .wanted_caps(["draft/chathistory", "labeled-response"]),
        );
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * LS :draft/chathistory labeled-response\r\n",
        );
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response\r\n",
        );
        while session.poll_transmit().is_some() {}
        session.handle_bytes(now, b":ergo.test 005 papo MSGREFTYPES=msgid :supported\r\n:ergo.test 001 papo :Welcome\r\n:ergo.test 422 papo :No MOTD\r\n");
        while session.poll_event().is_some() {}

        let defaults = super::super::PostConnect {
            bot: false,
            channels: vec!["#papo".into()],
            parted_while_disconnected: Vec::new(),
            history_on_reconnect: true,
            disconnected_at: Some(SystemTime::UNIX_EPOCH),
        };
        let mut catchups = BTreeMap::new();
        super::begin_catchup(&mut session, &defaults, &mut catchups);

        assert!(catchups.is_empty());
        assert!(session.poll_transmit().is_none());
    }

    #[test]
    fn reconnect_history_setting_controls_catchup() {
        for (enabled, expected_request) in [(false, false), (true, true)] {
            let mut session = registered_history_session();
            let mut catchups = BTreeMap::new();
            super::begin_catchup(&mut session, &reconnect_defaults(enabled), &mut catchups);

            assert_eq!(catchups.len(), usize::from(expected_request));
            assert_eq!(session.poll_transmit().is_some(), expected_request);
        }
    }
}
