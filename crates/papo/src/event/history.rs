use super::{
    CtcpQuery, CtcpReply, Incomplete, Joined, Kicked, Message, ModeChanged, NickChanged, Notice,
    Other, Parted, Quit, Reacted, Redacted, Restricted, RestrictedLine, ServerNotice, TagMsg,
    TopicChanged,
};

macro_rules! history_accessors {
    ($($ty:ident: |$this:ident| $history:expr),+ $(,)?) => {
        $(
            impl $ty {
                /// Returns whether the line came from a history page or a replay batch.
                ///
                /// A history line describes the past. Do not act on it as if it just happened.
                /// Unbatched replay arrives as live traffic and returns false, even with an old
                /// server timestamp.
                ///
                /// See [`crate::UserRef`] for sending to a history line's sender.
                #[must_use]
                pub const fn is_from_history(&$this) -> bool {
                    $history
                }
            }
        )+
    };
}

history_accessors!(
    Message: |self| self.receipt.is_from_history(),
    Notice: |self| self.receipt.is_from_history(),
    CtcpQuery: |self| self.receipt.is_from_history(),
    CtcpReply: |self| self.receipt.is_from_history(),
    ServerNotice: |self| self.receipt.is_from_history(),
    TagMsg: |self| self.receipt.is_from_history(),
    Reacted: |self| self.receipt.is_from_history(),
    Redacted: |self| self.receipt.is_from_history(),
    Joined: |self| self.from_history,
    Parted: |self| self.from_history,
    Quit: |self| self.from_history,
    Kicked: |self| self.from_history,
    NickChanged: |self| self.from_history,
    TopicChanged: |self| self.from_history,
    ModeChanged: |self| self.from_history,
    Other: |self| self.from_history,
    Incomplete: |self| match self {
        Self::Message(message) => message.is_from_history(),
        Self::Notice(notice) => notice.is_from_history(),
    },
    Restricted: |self| self.line.is_from_history(),
);

impl RestrictedLine {
    /// Returns whether the enclosed line came from a history page or a replay batch.
    ///
    /// See [`Message::is_from_history`] for how to handle replay and sending to its sender.
    #[must_use]
    pub const fn is_from_history(&self) -> bool {
        match self {
            Self::Message(line) => line.is_from_history(),
            Self::Notice(line) => line.is_from_history(),
            Self::Incomplete(line) => line.is_from_history(),
            Self::ServerNotice(line) => line.is_from_history(),
            Self::CtcpQuery(line) => line.is_from_history(),
            Self::CtcpReply(line) => line.is_from_history(),
            Self::TagMsg(line) => line.is_from_history(),
            Self::Reacted(line) => line.is_from_history(),
            Self::Redacted(line) => line.is_from_history(),
            // Typing notifications originate from live lines or local deadlines.
            Self::TypingChanged(_) => false,
            Self::Other(line) => line.is_from_history(),
        }
    }
}

#[cfg(test)]
#[cfg(not(browser))]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use papo_core::{Config, Session};

    use crate::Event;
    use crate::event_impl::{CoreEvent, from_core, incoming_from_core};

    const ERGO: &str = "ergo.test";
    const SOJU: &str = "soju.example";

    fn session(now: Instant) -> Session {
        let mut session = Session::new(
            Config::default()
                .nick("papo")
                .batch_timeout(Duration::from_secs(1)),
        );
        for line in [
            ":ergo.test 005 papo CASEMAPPING=rfc1459 CHANTYPES=# STATUSMSG=@+ PREFIX=(ov)@+ :are supported",
            ":papo!~papo@localhost JOIN #review",
            ":emily!~emily@cloak.example JOIN #review",
        ] {
            session.handle_line(now, line.as_bytes());
        }
        while session.poll_event().is_some() {}
        session
    }

    fn map_live(session: &Session, event: papo_core::Event) -> Option<Event> {
        let origin = crate::Origin::NONE;
        let isupport = session.isupport();
        let event = match from_core(event, isupport.casemapping(), origin, false, UNIX_EPOCH) {
            CoreEvent::Forward(event) => event,
            CoreEvent::Line(line) => {
                let (line, incomplete) = line.into_parts();
                let event = incoming_from_core(line, origin, isupport, false, UNIX_EPOCH, None);
                if incomplete {
                    super::super::mark_incomplete(event)?
                } else {
                    event
                }
            }
            _ => return None,
        };
        Some(super::super::separate_restricted(event, "papo", isupport))
    }

    type Accessor = fn(&Event) -> Option<bool>;

    fn assert_provenance(server: &str, batch: &str, line: &str, accessor: Accessor) {
        let now = Instant::now();
        let mut session = session(now);
        session.handle_line(now, line.as_bytes());
        let events: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
        let live_provenance = events
            .into_iter()
            .filter_map(|event| map_live(&session, event))
            .find_map(|event| accessor(&event));
        assert_eq!(live_provenance, Some(false), "{line}");

        session.handle_line(
            now,
            format!(":{server} BATCH +history {batch} #review").as_bytes(),
        );
        let tagged = line.strip_prefix('@').map_or_else(
            || format!("@batch=history {line}"),
            |rest| format!("@batch=history;{rest}"),
        );
        session.handle_line(now, tagged.as_bytes());
        session.handle_line(now, format!(":{server} BATCH -history").as_bytes());
        let Some(papo_core::Event::History { items, .. }) = session.poll_event() else {
            panic!("expected a replay page for {line}");
        };
        let [item] = items.try_into().expect("one history item");
        let (client, _) = crate::driver::test_support::client_with_commands(&session);
        let event = crate::ext::history::history_item(item, None, &client, UNIX_EPOCH);
        assert_eq!(accessor(&event), Some(true), "{line}: {event:?}");
    }

    macro_rules! accessor {
        ($variant:ident) => {
            |event: &Event| match event {
                Event::$variant(payload) => Some(payload.is_from_history()),
                _ => None,
            }
        };
        (restricted $variant:ident) => {
            |event: &Event| match event {
                Event::$variant(payload) => {
                    let line = crate::event::RestrictedLine::$variant(payload.clone());
                    let provenance = payload.is_from_history();
                    assert_eq!(line.is_from_history(), provenance);
                    Some(provenance)
                }
                _ => None,
            }
        };
    }

    #[test]
    fn chat_provenance_follows_the_batch_even_without_a_timestamp() {
        let rows: &[(&str, &str, Accessor)] = &[
            (
                ERGO,
                "@time=2026-10-19T14:31:11.456Z;msgid=RfcBTzyxY120005 :emily!~emily@localhost PRIVMSG #review :looks good",
                accessor!(restricted Message),
            ),
            (
                SOJU,
                "@time=2026-10-20T18:44:00.250Z :emily!~e@cloak.example NOTICE #review :build 4412 finished",
                accessor!(restricted Notice),
            ),
            (
                ERGO,
                ":emily!~emily@localhost PRIVMSG #review :\x01VERSION\x01",
                accessor!(restricted CtcpQuery),
            ),
            (
                SOJU,
                ":emily!~e@cloak.example NOTICE #review :\x01VERSION weechat 4.6.3\x01",
                accessor!(restricted CtcpReply),
            ),
            (
                ERGO,
                ":ergo.test NOTICE #review :Channel is now closed for maintenance",
                accessor!(restricted ServerNotice),
            ),
            (
                SOJU,
                "@+example/status=ready :emily!~e@cloak.example TAGMSG #review",
                accessor!(restricted TagMsg),
            ),
            (
                ERGO,
                "@+draft/react=eyes;+reply=RfcBTzyxY120004 :emily!~emily@localhost TAGMSG #review",
                accessor!(restricted Reacted),
            ),
            (
                SOJU,
                ":soju.example REDACT #review RfcBTzyxY120004 :sent too early",
                accessor!(restricted Redacted),
            ),
        ];
        for &(server, line, accessor) in rows {
            assert_provenance(server, "chathistory", line, accessor);
        }
    }

    #[test]
    fn activity_provenance_covers_users_and_servers() {
        let rows: &[(&str, Accessor)] = &[
            (
                ":alice!~alice@localhost JOIN #review alice :Alice Example",
                accessor!(Joined),
            ),
            (
                ":emily!~emily@localhost PART #review :See you tomorrow",
                accessor!(Parted),
            ),
            (
                ":emily!~emily@localhost QUIT :Connection reset by peer",
                accessor!(Quit),
            ),
            (
                ":ergo.test KICK #review emily :Channel closed",
                accessor!(Kicked),
            ),
            (
                ":emily!~emily@localhost NICK emily_",
                accessor!(NickChanged),
            ),
            (
                ":soju.example TOPIC #review :Release review",
                accessor!(TopicChanged),
            ),
            (":soju.example MODE #review +im", accessor!(ModeChanged)),
        ];
        for &(line, accessor) in rows {
            assert_provenance(SOJU, "draft/event-playback", line, accessor);
        }
    }

    #[test]
    fn untyped_lines_keep_known_replay_context() {
        for line in [
            ":soju.example TAGMSG #review",
            "NOTICE #review :Channel maintenance starts shortly",
        ] {
            assert_provenance(SOJU, "chathistory", line, accessor!(restricted Other));
        }

        let now = Instant::now();
        for (batch, historical) in [
            ("znc.in/playback", true),
            ("chathistory", true),
            ("draft/chathistory", true),
            ("draft/event-playback", true),
            ("vendor/sync", false),
        ] {
            let mut session = Session::new(Config::default().nick("papo").max_open_batches(0));
            session.handle_line(
                now,
                format!(":znc.example BATCH +replay {batch} #review").as_bytes(),
            );
            session.handle_line(now, b"@batch=replay;znc.in/server-time-iso=2026-10-19T14:31:11.456Z :emily!~e@localhost PRIVMSG #review :looks good");
            let events: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
            let provenance = events
                .into_iter()
                .filter_map(|event| map_live(&session, event))
                .find_map(|event| (accessor!(Other))(&event));
            assert_eq!(provenance, Some(historical), "{batch}");
        }
    }

    #[test]
    fn incomplete_chat_keeps_the_provenance_of_each_partial_payload() {
        for command in ["PRIVMSG", "NOTICE"] {
            for historical in [false, true] {
                let now = Instant::now();
                let mut session = session(now);
                let open = if historical {
                    session.handle_line(now, b":ergo.test BATCH +history chathistory #review");
                    "@batch=history :ergo.test BATCH +multi draft/multiline #review"
                } else {
                    ":ergo.test BATCH +multi draft/multiline #review"
                };
                session.handle_line(now, open.as_bytes());
                let line = format!(
                    "@batch=multi :emily!~emily@localhost {command} #review :review starts at"
                );
                session.handle_line(now, line.as_bytes());
                let timeout = papo_core::deadline(now, Duration::from_secs(2)).unwrap();
                session.handle_timeout(timeout);
                let events: Vec<_> = std::iter::from_fn(|| session.poll_event()).collect();
                let (client, _) = crate::driver::test_support::client_with_commands(&session);
                let provenance = events.into_iter().find_map(|event| {
                    let event = match event {
                        papo_core::Event::History { items, .. } => {
                            let [item] = items.try_into().expect("one partial item");
                            crate::ext::history::history_item(item, None, &client, UNIX_EPOCH)
                        }
                        event => map_live(&session, event)?,
                    };
                    (accessor!(restricted Incomplete))(&event)
                });
                assert_eq!(provenance, Some(historical), "{command}");
            }
        }
    }

    #[test]
    fn restricted_history_keeps_its_audience_and_provenance() {
        assert_provenance(
            SOJU,
            "chathistory",
            ":emily!~emily@cloak.example PRIVMSG @#review :ops review starts at noon",
            |event| match event {
                Event::Restricted(payload) => {
                    assert_eq!(payload.audience.symbol(), '@');
                    Some(payload.is_from_history())
                }
                _ => None,
            },
        );
        let now = Instant::now();
        let mut session = session(now);
        session.handle_line(
            now,
            b"@+typing=active :emily!~emily@cloak.example TAGMSG @#review",
        );
        let event = session.poll_event().unwrap();
        let Some(Event::Restricted(payload)) = map_live(&session, event) else {
            panic!("expected restricted typing notification");
        };
        assert!(matches!(
            payload.line,
            crate::event::RestrictedLine::TypingChanged(_)
        ));
        assert!(!payload.is_from_history());
    }
}
