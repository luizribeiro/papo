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
        let echo = match &response {
            Event::Response(resolved) => resolved.lines.last().cloned(),
            _ => None,
        }
        .unwrap_or_else(|| message.to_owned());
        let received = self.received_line(echo, None);
        self.outbox.event(Event::Line(received));
        // Consumers must observe the confirming event before the action completes.
        self.outbox.event(response);
    }
}

impl crate::SentEcho {
    /// Whether `message` is held until the line that completes its response arrives.
    pub(super) fn holds(&self, message: &LineRef<'_>) -> bool {
        match self.command {
            // Ergo sends a REDACT echo outside the labelled response, so its ACK may come first.
            Command::Redact => message.command_is("ACK") && self.reply.is_none(),
            _ => message.command() == Command::Numeric(Numeric::RPL_AWAY),
        }
    }
}

impl Session {
    /// Ties an unlabelled echo of our `REDACT` to the pending request that sent it.
    ///
    /// A message is redacted only once, so the echo from our nick naming its target and msgid is
    /// the one our request caused, unless our request then fails.
    pub(super) fn observe_redaction_echo(&mut self, message: &LineRef<'_>) {
        if message.command() != Command::Redact
            || message.tag("label").is_some()
            || !self.is_own_source(message)
        {
            return;
        }
        let mapping = self.isupport.value().casemapping();
        let Some(label) = self
            .requests
            .iter()
            .filter(|(_, request)| {
                request.echo.as_ref().is_some_and(|echo| {
                    echo.matches(message, mapping)
                        && echo
                            .reply
                            .as_ref()
                            .is_none_or(|reply| reply.as_ref().command_is("ACK"))
                })
            })
            .min_by_key(|(_, request)| request.id)
            .map(|(label, _)| label.clone())
        else {
            return;
        };
        let Some(echo) = self
            .requests
            .get_mut(&label)
            .and_then(|request| request.echo.as_mut())
        else {
            return;
        };
        if echo.reply.is_none() {
            echo.reply = Some(message.to_owned());
            return;
        }
        let Some(request) = self.requests.remove(&label) else {
            return;
        };
        let enabled_features = self.supported_features();
        self.outbox.event(Event::Response(crate::Response {
            id: request.id,
            sent: crate::sent_info(message),
            lines: vec![message.to_owned()],
            history: None,
            history_targets: None,
            whois_channels: Vec::new(),
            mode_list: None,
            state_limit: None,
            enabled_features,
            subject_tokens: Vec::new(),
        }));
    }

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
        let actor_party = self.actor_party(&line.as_ref());
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
                crate::Conversation::Private(nick) => self.state.__reply_token_for(nick.as_str()),
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
        let source_token = self.state.source_token(&line.as_ref());
        ReceivedLine {
            line,
            batch_type,
            actor_party,
            conversation,
            status,
            channel_context,
            reply_token,
            source_token,
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
        let context = claimed_channel_context(
            message,
            conversation?,
            self.isupport.value(),
            self.config.origin,
        )?;
        let papo_proto::Source::User { nick, .. } = message.source()? else {
            return None;
        };
        self.state.channel(&context)?.member(nick)?;
        Some(context)
    }
}

/// Returns the valid channel a private message's `+channel-context` tag names.
///
/// It is only the line's claim: the sender's membership in that channel is not checked.
#[doc(hidden)]
#[must_use]
pub fn claimed_channel_context(
    message: &LineRef<'_>,
    conversation: &crate::Conversation,
    isupport: &papo_proto::Isupport,
    origin: papo_proto::Origin,
) -> Option<crate::ChannelName> {
    if !matches!(conversation, crate::Conversation::Private(_)) {
        return None;
    }
    let context = message
        .tag("+channel-context")
        .or_else(|| message.tag("+draft/channel-context"))?;
    let context = crate::name::Names::new(isupport.casemapping(), origin).channel(context);
    context.check(isupport).ok()?;
    Some(context)
}
