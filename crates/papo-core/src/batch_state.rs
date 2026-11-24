use papo_proto::{Command, LineRef, Numeric};
use web_time::Instant;

use super::batch::{DispatchContext, RoutedMember};
use super::{Event, Session};

impl Session {
    pub(super) fn dispatch_batched_state(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
    ) -> Option<RoutedMember> {
        if message.command_is("BATCH") {
            return None;
        }
        let member = self.batches.live_member(message)?;
        if !updates_presence(message.command()) {
            self.observe_buffered_identity(message);
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
        self.dispatch_message(now, message, &context, emit_unhandled && !raw, enclosing);
        if !emit_events {
            self.outbox.events.truncate(before);
        }
        if raw {
            let received =
                self.received_line(message.to_owned(), context.batch_type().map(str::to_owned));
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
        })
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
