use crate::{Event, LineRef, Message, ReceivedMessage, Session, cap};

impl Session {
    pub(super) fn enqueue_labelled_echo(&mut self, message: &LineRef<'_>) {
        let response = self
            .outbox
            .events
            .pop_back()
            .expect("a resolved request queued its response");
        debug_assert!(matches!(response, Event::Response(_)));
        let received = self.received_message(message.to_owned(), None, true);
        self.outbox.event(Event::Message(received));
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
            self.isupport
                .value()
                .casemapping()
                .eq(nick, self.registration.nick())
        })
    }

    pub(super) fn received_message(
        &self,
        message: Message,
        batch_type: Option<String>,
        labelled_echo: bool,
    ) -> ReceivedMessage {
        let own_source = self.is_own_source(&message.as_ref());
        let labels = self.caps.enabled().contains_key(cap::LABELED_RESPONSE);
        ReceivedMessage {
            message,
            batch_type,
            echo: own_source && (labelled_echo || !labels),
            other_session: own_source && !labelled_echo && labels,
        }
    }
}
