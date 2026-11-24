use papo_proto::{Command, ModeChanges, ModeKind, Numeric};

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

impl Session {
    /// Ties an unlabelled echo of our `REDACT` to the pending request that sent it.
    ///
    /// A message is redacted only once, so the echo from our nick naming its target and msgid is
    /// the one our request caused, unless our request then fails. Ergo sends this echo just
    /// before the request's `ACK`, and the `ACK` ends the request whether or not it came.
    pub(super) fn observe_redaction_echo(&mut self, message: &LineRef<'_>) {
        if message.command() != Command::Redact
            || message.tag("label").is_some()
            || !self.is_own_source(message)
        {
            return;
        }
        let mapping = self.isupport.value().casemapping();
        if let Some(echo) = self
            .requests
            .values_mut()
            .filter(|request| {
                request
                    .echo
                    .as_ref()
                    .is_some_and(|echo| echo.reply.is_none() && echo.matches(message, mapping))
            })
            .min_by_key(|request| request.id)
            .and_then(|request| request.echo.as_mut())
        {
            echo.reply = Some(message.to_owned());
        }
    }

    /// Whether `message` is the server's echo of something we sent.
    pub(super) fn is_own_echo(&self, message: &LineRef<'_>) -> bool {
        crate::labels::is_echoed(message.command()) && self.is_own_source(message)
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
        for index in self.channel_key_params(&line.as_ref()) {
            line.__mark_secret(index);
        }
    }

    pub(super) fn mask_tracked_channel_keys(&self, message: &LineRef<'_>) -> Option<Line> {
        if message.command() != Command::Numeric(Numeric::RPL_CHANNELMODEIS) {
            return None;
        }
        let mut line = None;
        for index in self.channel_key_params(message) {
            if message
                .param(index)
                .is_some_and(|key| self.reply_secrets.contains(key))
            {
                papo_proto::__mask_param(line.get_or_insert_with(|| message.to_owned()), index);
            }
        }
        line
    }

    fn channel_key_params(&self, message: &LineRef<'_>) -> Vec<usize> {
        let Some((modestring, args_start)) = self.channel_mode_fields(message) else {
            return Vec::new();
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
        .collect()
    }

    pub(super) fn without_ambiguous_mode_args(&self, message: &LineRef<'_>) -> Option<Line> {
        let (modes, start) = self.channel_mode_fields(message)?;
        let isupport = self.isupport.value();
        let mut first_unknown = None;
        let mut next_arg = start;
        for change in ModeChanges::parse(modes, message.params().skip(start), isupport) {
            // A mode without advertised arity can shift any later argument onto a key.
            if change.mode == 'k'
                && let Some(offset) = first_unknown
            {
                return Some(papo_proto::__line_with_params_prefix(message, offset));
            }
            if change.kind == ModeKind::Unknown {
                first_unknown.get_or_insert(next_arg);
            }
            next_arg += usize::from(change.arg.is_some());
        }
        None
    }

    fn channel_mode_fields<'a>(&self, message: &LineRef<'a>) -> Option<(&'a str, usize)> {
        match message.command() {
            Command::Mode
                if message
                    .param(0)
                    .is_some_and(|target| self.isupport.value().is_channel(target)) =>
            {
                Some((message.param(1)?, 2))
            }
            Command::Numeric(Numeric::RPL_CHANNELMODEIS) => Some((message.param(2)?, 3)),
            _ => None,
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

pub(super) fn malformed_mode_may_carry_key(line: &[u8]) -> bool {
    let mut fields = line
        .split(|&byte| byte == b' ')
        .filter(|field| !field.is_empty())
        .peekable();
    for prefix in *b"@:" {
        if fields
            .peek()
            .is_some_and(|field| field.starts_with(&[prefix]))
        {
            fields.next();
        }
    }
    let Some(command) = fields.next() else {
        return false;
    };
    let target_fields = if command.eq_ignore_ascii_case(b"MODE") {
        1
    } else if command == b"324" {
        2
    } else {
        return false;
    };
    fields
        .nth(target_fields)
        .is_some_and(|modes| modes.contains(&b'k'))
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
