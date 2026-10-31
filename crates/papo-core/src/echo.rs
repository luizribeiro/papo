use papo_proto::{Command, ModeChanges, Numeric};

use crate::{Event, Line, LineRef, ReceivedLine, Session, conversation::classify_line_with_names};

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

    pub(super) fn received_line(&self, mut line: Line, batch_type: Option<String>) -> ReceivedLine {
        self.mark_channel_key(&mut line);
        let authorship = self.authorship(&line.as_ref());
        let (status, conversation) = classify_line_with_names(
            &line.as_ref(),
            self.nick(),
            self.isupport.value(),
            self.names(),
        )
        .map_or((None, None), |(status, target)| {
            (status, target.into_conversation())
        });
        let channel_context = self.verified_channel_context(&line.as_ref(), conversation.as_ref());
        let reply_token = conversation.as_ref().map_or_else(
            || self.state.__reply_token(),
            |conversation| match conversation {
                crate::Conversation::Private(nick) => self.state.__reply_token_for(nick),
                crate::Conversation::Channel(_) => {
                    if let Some(papo_proto::Source::User { nick, .. }) = line.as_ref().source() {
                        self.state.__reply_token_for(nick)
                    } else {
                        self.state.__reply_token()
                    }
                }
            },
        );
        let channel_guard = conversation
            .as_ref()
            .and_then(|conversation| match conversation {
                crate::Conversation::Channel(channel) => Some(self.channel_guard(channel)),
                crate::Conversation::Private(_) => None,
            });
        ReceivedLine {
            line,
            batch_type,
            authorship,
            conversation,
            status,
            channel_context,
            reply_token,
            channel_guard,
        }
    }

    pub(super) fn mark_channel_key(&self, line: &mut Line) {
        let key_params = {
            let message = line.as_ref();
            let (modestring, args_start) = match message.command() {
                Command::Mode
                    if message
                        .param(0)
                        .is_some_and(|target| self.isupport.value().is_channel(target)) =>
                {
                    (message.param(1), 2)
                }
                Command::Numeric(Numeric::RPL_CHANNELMODEIS) => (message.param(2), 3),
                _ => return,
            };
            let Some(modestring) = modestring else {
                return;
            };
            let mut consumed = 0;
            ModeChanges::parse(
                modestring,
                message.params().skip(args_start),
                self.isupport.value(),
            )
            .filter_map(|change| {
                let index = args_start + consumed;
                if change.arg.is_some() {
                    consumed += 1;
                }
                (change.mode == 'k' && message.param(index).is_some()).then_some(index)
            })
            .collect::<Vec<_>>()
        };
        for index in key_params {
            line.__mark_secret(index);
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
        let context = self.names().channel(context);
        context.check(self.isupport.value()).ok()?;
        let papo_proto::Source::User { nick, .. } = message.source()? else {
            return None;
        };
        self.state.channel(&context)?.member(nick)?;
        Some(context)
    }
}
