use papo_proto::{Command, LineRef, Numeric};
use web_time::Instant;

use super::batch::{DispatchContext, RoutedMember};
use super::{Event, IncomingLine, Session};

impl Session {
    pub(super) fn dispatch_batched_state(
        &mut self,
        now: Instant,
        incoming: IncomingLine<'_>,
    ) -> Option<RoutedMember> {
        let message = &incoming.shown;
        if message.command_is("BATCH") {
            return None;
        }
        let member = self.batches.live_member(message)?;
        if !updates_presence(message.command()) {
            self.observe_batched_identity(message);
            return None;
        }
        let mut context = DispatchContext::from_batch_type(member.kind.to_owned());
        let request = member.label.and_then(|label| self.requests.get(label));
        let emit_unhandled = request.is_none_or(|request| !self.answers_request(request, message));
        let raw = member.raw && request.is_none();
        let emit_events =
            !raw && (emit_unhandled || request.is_some_and(|request| request.emit_events));
        let who_target = request.and_then(|request| request.who_target.clone());
        let enclosing = request.map(|request| (request.id, who_target.as_deref()));
        context.defer_responses = raw || enclosing.is_some();
        context.live_input = true;
        let split = member.split;
        let before = self.outbox.events.len();

        // The application can send between any two server lines.
        self.dispatch_message(now, incoming, &context, emit_unhandled && !raw, enclosing);
        if !emit_events {
            self.outbox.events.truncate(before);
        }
        if raw {
            let received = self.received_line(
                incoming.shown.to_owned(),
                context.batch_type().map(str::to_owned),
            );
            self.outbox.event(Event::Line(received));
        }
        if let Some(split) = split {
            for event in self.outbox.events.range_mut(before..) {
                match event {
                    Event::Quit { netsplit, .. } => *netsplit = Some(split.clone()),
                    Event::Joined { netjoin, .. } => *netjoin = Some(split.clone()),
                    _ => {}
                }
            }
        }
        let joined_ready = message
            .param(0)
            .filter(|_| message.command() == Command::Join)
            .and_then(|channel| self.state.channel(channel))
            .map(crate::Channel::is_synced);
        Some(RoutedMember {
            live: true,
            replay_copy: false,
            subject: self.state.who_subject_token(message),
            joined_ready,
            sender: None,
        })
    }

    pub(super) fn observe_multiline_opener(&mut self, message: &LineRef<'_>) {
        let Some((reference, target)) = self.batches.live_multiline_opener(message) else {
            return;
        };
        let (Some(papo_proto::Source::User { .. }), Some(source)) =
            (message.source(), message.raw_source())
        else {
            return;
        };
        // The server supplies the sender and account for the message on this opener.
        let mut opening = crate::Line::privmsg(target, "").with_source(source);
        for (key, value) in message.tags() {
            opening = opening.with_tag(key, value.as_deref().unwrap_or_default());
        }
        self.observe_buffered_identity(&opening.as_ref(), false);
        self.batches
            .remember_multiline_sender(reference, self.state.source_token(message));
    }

    fn observe_batched_identity(&mut self, message: &LineRef<'_>) {
        let Some((reference, opening)) = self.batches.multiline_opening(message) else {
            self.observe_buffered_identity(message, true);
            return;
        };
        // The opener carries the account tag for the whole message. Later fragments repeat the
        // source but provide no new identity evidence for the assembled message.
        if let Some(opening) = opening {
            if self
                .batches
                .multiline_sender(reference)
                .is_some_and(|sender| sender != self.state.source_token(message))
            {
                return;
            }
            let reference = reference.to_owned();
            let account = opening.as_ref().tag("account");
            if let Some(account) = account {
                let part = message.to_owned().with_tag("account", account);
                self.observe_buffered_identity(&part.as_ref(), true);
            } else {
                self.observe_buffered_identity(message, true);
            }
            self.batches
                .remember_multiline_sender(&reference, self.state.source_token(message));
        }
    }
}

fn updates_presence(command: Command<'_>) -> bool {
    matches!(
        command,
        Command::Quit
            | Command::Nick
            | Command::Part
            | Command::Kick
            | Command::Join
            | Command::Account
            | Command::Chghost
            | Command::Mode
            | Command::Away
            | Command::Setname
            | Command::Rename
            | Command::Topic
            | Command::Markread
            | Command::Numeric(
                Numeric::RPL_NAMREPLY
                    | Numeric::RPL_ENDOFNAMES
                    | Numeric::RPL_MONONLINE
                    | Numeric::RPL_MONOFFLINE
                    | Numeric::RPL_WHOREPLY
                    | Numeric::RPL_WHOSPCRPL
                    | Numeric::RPL_ENDOFWHO
                    | Numeric::RPL_WHOISUSER
                    | Numeric::RPL_WHOISBOT
                    | Numeric::RPL_VISIBLEHOST
                    | Numeric::RPL_AWAY
                    | Numeric::RPL_UNAWAY
                    | Numeric::RPL_NOWAWAY
                    | Numeric::RPL_UMODEIS
                    | Numeric::RPL_CHANNELMODEIS
                    | Numeric::RPL_CREATIONTIME
                    | Numeric::RPL_TOPIC
                    | Numeric::RPL_TOPICWHOTIME
            )
    )
}
