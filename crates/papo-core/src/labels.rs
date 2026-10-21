use crate::{
    Command, Event, Feature, Instant, Line, LineRef, MAX_PENDING_REQUESTS, Numeric, RequestError,
    RequestId, Response, ResponseMatcher, Session, cap, parse_whois_channels, sent_info,
};

impl Session {
    pub(super) fn resolve_message(&mut self, message: &LineRef<'_>) -> bool {
        let Some(label) = message.tag("label") else {
            return false;
        };
        let Some((id, _, sent, _, _)) = self.requests.remove(label.as_ref()) else {
            return false;
        };
        self.away_messages.remove(&id);
        let lines = if message.command_is("ACK") {
            Vec::new()
        } else {
            vec![message.to_owned()]
        };
        let whois_channels = parse_whois_channels(&lines, self.isupport.value());
        if !self
            .unlabelled_requests
            .iter()
            .any(|request| request.id == id)
        {
            self.outbox.event(Event::Response(Response {
                id,
                sent: if sent { sent_info(message) } else { None },
                lines,
                history: None,
                history_targets: None,
                whois_channels,
            }));
        }
        true
    }

    pub(super) fn resolve_labelled_message(
        &mut self,
        message: &LineRef<'_>,
        queued_events: usize,
        emit_events: bool,
    ) -> bool {
        let resolved = self.resolve_message(message);
        if resolved && !emit_events {
            let response = self
                .outbox
                .events
                .pop_back()
                .expect("a resolved request queued its response");
            self.outbox.events.truncate(queued_events);
            self.outbox.event(response);
        }
        resolved
    }

    pub(super) fn response_emits_events(&self, message: &LineRef<'_>) -> bool {
        message
            .tag("label")
            .and_then(|label| self.requests.get(label.as_ref()))
            .is_none_or(|(_, _, _, _, emit_events)| *emit_events)
    }

    pub(super) fn confirmed_away_message<'a>(&'a self, message: &LineRef<'_>) -> Option<&'a str> {
        if message.command() != Command::Numeric(Numeric::RPL_NOWAWAY) {
            return None;
        }
        let id = if let Some(label) = message.tag("label") {
            self.requests.get(label.as_ref()).map(|(id, ..)| *id)
        } else {
            self.unlabelled_requests.iter().find_map(|request| {
                matches!(&request.matcher, ResponseMatcher::Away(true)).then_some(request.id)
            })
        }?;
        self.away_messages.get(&id).map(String::as_str)
    }

    pub(super) fn resolve_unlabelled(&mut self, now: Instant, message: &LineRef<'_>) -> bool {
        let mapping = self.isupport.value().casemapping();
        let Some((index, complete, join_started)) = self
            .unlabelled_requests
            .iter()
            .enumerate()
            .find_map(|(index, request)| {
                let join_started = matches!(
                    &request.matcher,
                    ResponseMatcher::Join(channel)
                        if message.command() == Command::Join
                            && message
                                .param(0)
                                .is_some_and(|target| crate::names_equal(mapping, target, channel))
                            && self.is_own_source(message)
                );
                let fence = request.fence.as_ref().and_then(|token| {
                    (message.command() == Command::Pong
                        && message.params().last() == Some(token.as_str()))
                    .then_some(true)
                });
                fence
                    .or_else(|| {
                        request
                            .matcher
                            .matches(message, mapping, self.welcome_state.nick(), &self.state)
                            .map(|complete| complete && request.fence.is_none())
                    })
                    .map(|done| (index, done, join_started))
            })
        else {
            return false;
        };
        let request = self.unlabelled_requests.get_mut(index).unwrap();
        if join_started {
            request.started = Some(now);
        }
        request.lines.push(message.to_owned());
        if complete {
            let request = self.unlabelled_requests.remove(index).unwrap();
            self.away_messages.remove(&request.id);
            let whois_channels = parse_whois_channels(&request.lines, self.isupport.value());
            self.outbox.event(Event::Response(Response {
                id: request.id,
                sent: None,
                lines: request.lines,
                history: None,
                history_targets: None,
                whois_channels,
            }));
        }
        true
    }
}

impl Session {
    pub(super) fn queue_request(
        &mut self,
        now: Instant,
        mut message: Line,
        feature: Option<Feature>,
        emit_events: bool,
    ) -> Result<RequestId, RequestError> {
        self.rate_limit.advance(now);
        let id = RequestId(self.next_request_id);
        let sent = self.caps.enabled().contains_key(&cap::ECHO_MESSAGE)
            && matches!(
                message.as_ref().command(),
                Command::Privmsg | Command::Notice
            );
        let labelled = self.caps.enabled().contains_key(cap::LABELED_RESPONSE);
        let label = labelled.then(|| encode_label(self.next_request_id));
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        if let Some(label) = &label {
            message = message.with_tag("label", label);
        }
        let line = message.to_line()?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if let Some(label) = label {
            self.requests
                .insert(label, (id, None, sent, feature, emit_events));
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox.transmit_tracked(line.into(), None, Some(id));
        Ok(id)
    }
}

pub(super) fn encode_label(mut value: u64) -> String {
    const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut buffer = [0_u8; 11];
    let mut index = buffer.len();
    loop {
        index -= 1;
        buffer[index] = ALPHABET[(value % 62) as usize];
        value /= 62;
        if value == 0 {
            return String::from_utf8(buffer[index..].to_vec()).expect("labels are ASCII");
        }
    }
}
