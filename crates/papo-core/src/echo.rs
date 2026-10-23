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
        let channel_context = self.verified_channel_context(&line.as_ref(), conversation.as_ref());
        ReceivedLine {
            line,
            batch_type,
            authorship,
            conversation,
            status,
            channel_context,
            reply_token: self.state.reply_token(),
        }
    }

    fn verified_channel_context(
        &self,
        message: &LineRef<'_>,
        conversation: Option<&crate::Conversation>,
    ) -> Option<crate::ChannelName> {
        if !matches!(conversation, Some(crate::Conversation::Private(_))) {
            return None;
        }
        let context = message
            .tag("+channel-context")
            .or_else(|| message.tag("+draft/channel-context"))?;
        let context = self.isupport.value().casemapping().channel(context);
        context.check(self.isupport.value()).ok()?;
        let papo_proto::Source::User { nick, .. } = message.source()? else {
            return None;
        };
        self.state.channel(&context)?.member(nick)?;
        Some(context)
    }
}
