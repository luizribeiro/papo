use crate::{Event, Line, LineRef, ReceivedLine, Session, conversation::classify_line};

impl Session {
    pub(super) fn enqueue_labelled_echo(&mut self, message: &LineRef<'_>) {
        let response = self
            .outbox
            .events
            .pop_back()
            .expect("a resolved request queued its response");
        debug_assert!(matches!(response, Event::Response(_)));
        let received = self.received_line(message.to_owned(), None);
        self.outbox.event(Event::Line(received));
        // Consumers must observe the confirming event before the action completes.
        self.outbox.event(response);
    }
}

impl Session {
    pub(super) fn is_own_source(&self, message: &LineRef<'_>) -> bool {
        message.source().is_some_and(|source| {
            let papo_proto::Source::User { nick, .. } = source else {
                return false;
            };
            crate::names_equal(
                self.isupport.value().casemapping(),
                nick,
                self.welcome_state.nick(),
            )
        })
    }

    pub(super) fn received_line(&self, line: Line, batch_type: Option<String>) -> ReceivedLine {
        let authorship = self.authorship(&line.as_ref());
        let (status, conversation) =
            classify_line(&line.as_ref(), self.nick(), self.isupport.value())
                .map_or((None, None), |(status, conversation)| {
                    (status, Some(conversation))
                });
        ReceivedLine {
            line,
            batch_type,
            authorship,
            conversation,
            status,
        }
    }
}
