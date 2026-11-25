use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use super::{EventSink, PostConnect, now};
use crate::{Client, Event};
use papo_core::{HistoryQuery, RequestId, Response, Session};

const CATCHUP_ITEM_LIMIT: usize = 1_000;
const CATCHUP_PAGE_SIZE: usize = 100;

// A live msgid can only repeat a catchup item while fewer than the item limit precede it, and one
// more page may be in flight when it arrives.
const LIVE_SEEN_LIMIT: usize = CATCHUP_ITEM_LIMIT + CATCHUP_PAGE_SIZE;

pub(super) struct Catchup {
    pub(super) target: String,
    pub(super) items: Vec<papo_core::HistoryItem>,
    pub(super) seen: HashSet<String>,
}

#[derive(Clone, Debug)]
pub(super) struct CatchupCursor {
    msgid: Option<String>,
    time: crate::ServerTime,
}

pub(super) fn remember_live_event<'a>(
    catchups: impl IntoIterator<Item = &'a mut Catchup>,
    event: &Event,
) {
    let Some(msgid) = event.msgid() else {
        return;
    };
    for catchup in catchups {
        if catchup.seen.len() < LIVE_SEEN_LIMIT && event_applies_to_channel(event, &catchup.target)
        {
            catchup.seen.insert(msgid.to_string());
        }
    }
}

fn event_applies_to_channel(event: &Event, target: &str) -> bool {
    if matches!(event.conversation(), Some(crate::Conversation::Channel(channel)) if channel.name() == target)
    {
        return true;
    }
    match event {
        Event::ChannelRenamed(line) => line.old == target,
        Event::Quit(line) => line.channels.iter().any(|channel| channel.name() == target),
        Event::NickChanged(line) => line.channels.iter().any(|channel| channel.name() == target),
        Event::Other(line) => line
            .as_ref()
            .param(0)
            .is_some_and(|address| address.ends_with(target)),
        _ => false,
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
    let core_conversation = page.conversation;
    let exhausted = page.exhausted;
    let previous_len = catchup.items.len();
    let next = extend_catchup(&mut catchup, page.items);
    if !exhausted
        && catchup.items.len() > previous_len
        && catchup.items.len() < CATCHUP_ITEM_LIMIT
        && let Some(next) = next
    {
        let query = HistoryQuery::__after(
            catchup.target.clone(),
            papo_core::MsgRef::Msgid(next),
            (CATCHUP_ITEM_LIMIT - catchup.items.len()).min(CATCHUP_PAGE_SIZE),
        );
        if let Ok(id) = session.chathistory(now(), query) {
            catchups.insert(id, catchup);
            return None;
        }
    }
    let commands = events.commands.upgrade()?;
    if let Some(cursor) = newest_cursor(&catchup.items)
        && let Some(channel) = defaults.channels.iter_mut().find(|channel| {
            let mapping = session.isupport().casemapping();
            mapping.fold(&channel.channel) == mapping.fold(catchup.target.as_str())
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
        origin: session.state().nick().origin(),
        connection_checks: Arc::clone(&events.connection_checks),
        network_builder: None,
    };
    let conversation = {
        let state = client.state();
        crate::Conversation::from_core_with_state(
            core_conversation,
            client.origin(),
            &client.state,
            &state,
        )
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
    Some(Event::History(crate::event::HistoryPage {
        conversation,
        items,
        exhausted,
        reason: crate::HistoryReason::Catchup,
    }))
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
        if catchup.items.len() == CATCHUP_ITEM_LIMIT {
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
                .and_then(|value| papo_proto::ServerTime::from_stored(&value).ok())?;
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
        let Some(after) = cursor
            .and_then(|cursor| cursor.reference(supports_timestamp))
            .or_else(|| {
                seen.as_ref()
                    .map(|seen| papo_core::MsgRef::Msgid(seen.msgid.clone()))
            })
        else {
            continue;
        };
        let query = HistoryQuery::__after(channel.channel.clone(), after, CATCHUP_PAGE_SIZE);
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
                .wanted_caps(["draft/chathistory", "labeled-response", "batch"]),
        );
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch\r\n",
        );
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch\r\n",
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

    fn live_reaction(msgid: &str) -> crate::Event {
        let mapping = papo_proto::Casemapping::default();
        let actor = crate::__testing::user_handle(mapping.nick("emily"));
        let conversation = crate::Conversation::Channel(crate::ChannelRef::from_parts(
            mapping.channel("#papo"),
            None,
            None,
        ));
        let actor =
            crate::UserRef::current_name(actor.nick().clone(), actor.account().clone(), None, None);
        let receipt = crate::message_ref::Receipt::new(
            crate::Actor::User(actor.clone()),
            &conversation,
            None,
            "#papo".into(),
        );
        crate::Event::Reacted(crate::event::Reacted {
            actor_account: crate::AccountStatus::Unknown,
            actor_is_bot: None,
            msgid: Some(msgid.into()),
            server_time: None,
            received_at: SystemTime::UNIX_EPOCH,
            actor,
            actor_party: crate::Party::Other,
            conversation,
            status: None,
            receipt,
            reacts_to: "RfcBTzyxY120008".into(),
            reaction: "👍".into(),
            action: crate::event::ReactionAction::Added,
        })
    }

    fn papo_catchup() -> super::Catchup {
        super::Catchup {
            target: "#papo".into(),
            items: Vec::new(),
            seen: std::collections::HashSet::new(),
        }
    }

    #[test]
    fn live_msgids_stop_at_the_catchup_limit_plus_one_page() {
        let mut catchup = papo_catchup();
        for index in 0..3_000 {
            let live = live_reaction(&format!("RfcBTzyxY{index:06}"));
            super::remember_live_event(std::iter::once(&mut catchup), &live);
        }

        assert_eq!(catchup.seen.len(), 1_000 + 100);
        assert!(catchup.seen.contains("RfcBTzyxY001099"));
        assert!(!catchup.seen.contains("RfcBTzyxY001100"));
    }

    #[test]
    fn gap_fill_deduplicates_a_live_reaction_by_msgid() {
        let mut catchup = papo_catchup();
        let live = live_reaction("RfcBTzyxY120010");
        super::remember_live_event(std::iter::once(&mut catchup), &live);
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
            actor_party: papo_core::Party::Unknown,
            subject_party: papo_core::Party::Unknown,
        }
    }

    fn mapped_event(input: &str, variant: &str) -> crate::Event {
        let mut found = None;
        crate::__testing::exercise_session_mapping(input.as_bytes(), |event| {
            let name = match &event {
                crate::Event::Message(_) => "Message",
                crate::Event::Notice(_) => "Notice",
                crate::Event::CtcpQuery(_) => "CtcpQuery",
                crate::Event::CtcpReply(_) => "CtcpReply",
                crate::Event::TagMsg(_) => "TagMsg",
                crate::Event::Reacted(_) => "Reacted",
                crate::Event::Redacted(_) => "Redacted",
                crate::Event::ServerNotice(_) => "ServerNotice",
                crate::Event::Joined(_) => "Joined",
                crate::Event::Parted(_) => "Parted",
                crate::Event::Kicked(_) => "Kicked",
                crate::Event::Invited(_) => "Invited",
                crate::Event::ChannelRenamed(_) => "ChannelRenamed",
                crate::Event::ModeChanged(_) => "ModeChanged",
                crate::Event::TopicChanged(_) => "TopicChanged",
                crate::Event::Quit(_) => "Quit",
                crate::Event::NickChanged(_) => "NickChanged",
                crate::Event::Other(_) => "Other",
                _ => return,
            };
            if name == variant {
                found = Some(event);
            }
        });
        found.unwrap_or_else(|| panic!("{variant} event from {input:?}"))
    }

    fn assert_conversation_events_are_scoped() {
        let conversation_cases = [
            (
                "Message",
                "PRIVMSG #papo :release deployed",
                "PRIVMSG #rust :release deployed",
                "PRIVMSG papo :release deployed",
            ),
            (
                "Notice",
                "NOTICE #papo :maintenance starts soon",
                "NOTICE #rust :maintenance starts soon",
                "NOTICE papo :maintenance starts soon",
            ),
            (
                "CtcpQuery",
                "PRIVMSG #papo :\u{1}VERSION\u{1}",
                "PRIVMSG #rust :\u{1}VERSION\u{1}",
                "PRIVMSG papo :\u{1}VERSION\u{1}",
            ),
            (
                "CtcpReply",
                "NOTICE #papo :\u{1}VERSION papo 0.1\u{1}",
                "NOTICE #rust :\u{1}VERSION papo 0.1\u{1}",
                "NOTICE papo :\u{1}VERSION papo 0.1\u{1}",
            ),
            (
                "TagMsg",
                "+example.org/status=reviewing TAGMSG #papo",
                "+example.org/status=reviewing TAGMSG #rust",
                "+example.org/status=reviewing TAGMSG papo",
            ),
            (
                "Reacted",
                "+draft/react=👍;+draft/reply=RfcBTzyxY120001 TAGMSG #papo",
                "+draft/react=👍;+draft/reply=RfcBTzyxY120001 TAGMSG #rust",
                "+draft/react=👍;+draft/reply=RfcBTzyxY120001 TAGMSG papo",
            ),
            (
                "Redacted",
                "REDACT #papo RfcBTzyxY120001 :superseded",
                "REDACT #rust RfcBTzyxY120001 :superseded",
                "REDACT papo RfcBTzyxY120001 :superseded",
            ),
        ];
        for (variant, same, other, private) in conversation_cases {
            for (line, expected) in [(same, true), (other, false), (private, false)] {
                let input = if let Some((tags, command)) =
                    line.strip_prefix('+').and_then(|line| line.split_once(' '))
                {
                    format!("@msgid=RfcBTzyxY120010;+{tags} :emily!~e@cloak.example {command}\r\n")
                } else {
                    format!("@msgid=RfcBTzyxY120010 :emily!~e@cloak.example {line}\r\n")
                };
                let event = mapped_event(&input, variant);
                assert_eq!(
                    super::event_applies_to_channel(&event, "#papo"),
                    expected,
                    "{variant}: {line}"
                );
            }
        }
    }

    #[test]
    fn every_event_kind_is_scoped_to_its_catchup_channel() {
        assert_conversation_events_are_scoped();
        let channel_cases = [
            ("ServerNotice", ":ergo.test NOTICE #papo :Maintenance"),
            ("Joined", ":emily!e@cloak JOIN #papo * :Emily"),
            ("Parted", ":emily!~e@cloak.example PART #papo :Leaving"),
            ("Kicked", ":ChanServ KICK #papo emily :Requested"),
            ("Invited", ":emily!~e@cloak.example INVITE papo #papo"),
            (
                "ChannelRenamed",
                ":emily!e@cloak RENAME #papo #papo-dev :Renamed",
            ),
            (
                "ChannelRenamed",
                ":emily!e@cloak RENAME #legacy #papo :Renamed",
            ),
            ("ModeChanged", ":operator!~o@staff.example MODE #papo +m"),
            ("TopicChanged", ":emily!e@cloak TOPIC #papo :Releases"),
            (
                "Quit",
                ":papo!~p@client.example JOIN #papo * :Papo User\r\n:emily!~e@cloak.example JOIN #papo * :Emily Example\r\n@msgid=RfcBTzyxY120010 :emily!~e@cloak.example QUIT :Client exited",
            ),
            (
                "NickChanged",
                ":papo!~p@client.example JOIN #papo * :Papo User\r\n:emily!~e@cloak.example JOIN #papo * :Emily Example\r\n@msgid=RfcBTzyxY120010 :emily!~e@cloak.example NICK emily_",
            ),
            ("Other", ":ergo.test VENDOR @#papo :network-specific state"),
        ];
        for (variant, line) in channel_cases {
            let input = if line.contains("@msgid=") {
                format!("{line}\r\n")
            } else {
                format!("@msgid=RfcBTzyxY120010 {line}\r\n")
            };
            let event = mapped_event(&input, variant);
            assert!(
                super::event_applies_to_channel(&event, "#papo"),
                "{variant}: {line}"
            );
            assert!(
                !super::event_applies_to_channel(&event, "#rust"),
                "{variant}: {line}"
            );
            assert!(
                !super::event_applies_to_channel(&event, "emily"),
                "{variant}: {line}"
            );
        }
    }

    #[test]
    fn activity_without_a_msgid_advances_the_timestamp_cursor() {
        let time = papo_proto::ServerTime::from_stored("2026-10-15T12:01:00.000Z").unwrap();
        let item = HistoryItem::Joined(HistoryLine {
            line: LineRef::parse("@time=2026-10-15T12:01:00.000Z :papo!papo@localhost JOIN #papo")
                .unwrap()
                .to_owned(),
            context: false,
            actor_party: papo_core::Party::Me,
            subject_party: papo_core::Party::Unknown,
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
                .wanted_caps(["draft/chathistory", "labeled-response", "batch"]),
        );
        session.handle_connected(now);
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * LS :draft/chathistory labeled-response batch\r\n",
        );
        while session.poll_transmit().is_some() {}
        session.handle_bytes(
            now,
            b":ergo.test CAP * ACK :draft/chathistory labeled-response batch\r\n",
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
            session.handle_bytes(
                std::time::Instant::now(),
                b"@time=2026-11-08T19:55:04.123Z;msgid=RfcBTzyxY120004 :alice!a@cloak PRIVMSG #papo :checkpoint\r\n",
            );
            while session.poll_event().is_some() {}
            let mut catchups = BTreeMap::new();
            super::begin_catchup(&mut session, &reconnect_defaults(enabled), &mut catchups);

            assert_eq!(catchups.len(), usize::from(expected_request));
            assert_eq!(session.poll_transmit().is_some(), expected_request);
        }
    }

    #[test]
    fn reconnect_never_uses_the_device_disconnect_time_as_a_server_cursor() {
        let mut session = registered_history_session();
        let mut catchups = BTreeMap::new();
        super::begin_catchup(&mut session, &reconnect_defaults(true), &mut catchups);

        assert!(catchups.is_empty());
        assert!(session.poll_transmit().is_none());
    }

    #[test]
    fn every_request_end_drops_its_catchup() {
        use crate::event_impl::RequestEnd;
        let unsupported = || {
            papo_core::Unsupported::new(
                papo_core::Feature::History,
                papo_core::UnsupportedReason::NotOffered,
            )
        };
        for end in [
            RequestEnd::Untracked,
            RequestEnd::TimedOut,
            RequestEnd::Failed(papo_core::RequestError::PendingLimit),
            RequestEnd::Aborted { transmitted: true },
            RequestEnd::RecipientGone,
            RequestEnd::CredentialsRefused,
            RequestEnd::Unsupported(unsupported()),
        ] {
            let mut session = registered_history_session();
            session.handle_bytes(
                std::time::Instant::now(),
                b"@time=2026-11-08T19:55:04.123Z;msgid=RfcBTzyxY120004 :alice!a@cloak PRIVMSG #papo :checkpoint\r\n",
            );
            while session.poll_event().is_some() {}
            let mut catchups = BTreeMap::new();
            super::begin_catchup(&mut session, &reconnect_defaults(true), &mut catchups);
            let id = *catchups.keys().next().expect("catchup request");
            super::super::end_request(
                &mut session,
                &mut BTreeMap::new(),
                &mut BTreeMap::new(),
                &mut catchups,
                id,
                end,
            );

            assert!(catchups.is_empty());
        }
    }
}
