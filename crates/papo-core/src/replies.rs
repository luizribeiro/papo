use crate::{Event, LineRef, Redacted, Session, StandardReply, StandardReplyView, TagMessage};

impl Session {
    pub(super) fn report_tag_message(&mut self, message: &LineRef<'_>) -> bool {
        let Some(target) = message.param(0).filter(|_| message.command_is("TAGMSG")) else {
            return false;
        };
        self.outbox.event(Event::TagMessage(TagMessage {
            source: message.raw_source().map(str::to_owned),
            target: target.to_owned(),
            tags: message
                .tags()
                .map(|(key, value)| (key.to_owned(), value.map(std::borrow::Cow::into_owned)))
                .collect(),
        }));
        true
    }

    pub(super) fn report_redaction(&mut self, message: &LineRef<'_>) -> bool {
        let (Some(target), Some(msgid)) = (
            message.param(0).filter(|_| message.command_is("REDACT")),
            message.param(1),
        ) else {
            return false;
        };
        self.outbox.event(Event::Redacted(Redacted {
            source: message.raw_source().map(str::to_owned),
            target: target.to_owned(),
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
        let casemapping = self.isupport.value().casemapping();
        self.outbox.event(Event::Invited {
            channel: casemapping.channel(channel),
            nick: casemapping.nick(nick),
            by: by.to_owned(),
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
        }));
        true
    }
}
