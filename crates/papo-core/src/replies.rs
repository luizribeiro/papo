use crate::{
    Authorship, Event, LineEnvelope, LineRef, Redacted, Session, StandardReply, StandardReplyView,
    TagMessage,
    conversation::classify_line_with_names,
};

impl Session {
    pub(super) fn report_tag_message(&mut self, message: &LineRef<'_>) -> bool {
        if !message.command_is("TAGMSG") {
            return false;
        }
        let Some(actor) = message.raw_source() else {
            return false;
        };
        let Ok((status, target)) =
            classify_line_with_names(message, self.nick(), self.isupport.value(), self.names())
        else {
            return false;
        };
        let conversation = target.into_conversation();
        self.outbox.event(Event::TagMessage(TagMessage {
            envelope: LineEnvelope::from_line(message),
            actor: actor.to_owned(),
            authorship: self.authorship(message),
            conversation,
            status,
            tags: message
                .tags()
                .map(|(key, value)| (key.to_owned(), value.map(std::borrow::Cow::into_owned)))
                .collect(),
        }));
        true
    }

    pub(super) fn report_redaction(&mut self, message: &LineRef<'_>) -> bool {
        let (Some(actor), Some(msgid)) = (
            message.raw_source(),
            message.param(1).filter(|_| message.command_is("REDACT")),
        ) else {
            return false;
        };
        let Ok((status, target)) =
            classify_line_with_names(message, self.nick(), self.isupport.value(), self.names())
        else {
            return false;
        };
        let conversation = target.into_conversation();
        self.outbox.event(Event::Redacted(Redacted {
            envelope: LineEnvelope::from_line(message),
            actor: actor.to_owned(),
            authorship: self.authorship(message),
            conversation,
            status,
            msgid: msgid.to_owned(),
            reason: message.param(2).map(str::to_owned),
        }));
        true
    }

    pub(super) fn report_invite(&mut self, message: &LineRef<'_>) -> bool {
        let (Some(nick), Some(channel), Some(by)) = (
            message.param(0).filter(|_| message.command_is("INVITE")),
            message.param(1),
            message.raw_source(),
        ) else {
            return false;
        };
        let casemapping = self.names();
        self.outbox.event(Event::Invited {
            channel: casemapping.channel(channel),
            subject: casemapping.nick(nick),
            actor: by.to_owned(),
            authorship: self.authorship(message),
            subject_authorship: if crate::names_equal(casemapping, nick, self.nick()) {
                Authorship::Own
            } else {
                Authorship::Other
            },
        });
        true
    }
}

impl Session {
    pub(super) fn report_standard_reply(&mut self, message: &LineRef<'_>) -> bool {
        let Some(reply) = StandardReplyView::parse(message) else {
            return false;
        };
        self.outbox.event(Event::StandardReply(StandardReply {
            kind: reply.kind,
            command: reply.command.to_ascii_uppercase(),
            code: reply.code.to_ascii_uppercase(),
            context: reply.context.into_iter().map(str::to_owned).collect(),
            description: reply.description.to_owned(),
            #[cfg(feature = "unstable")]
            refusal: crate::Refusal::from_standard_enabled(
                reply.command,
                reply.code,
                &self.supported_features(),
            ),
        }));
        true
    }
}
