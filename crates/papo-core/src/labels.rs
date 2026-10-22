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
        if message.command_is("ACK") && self.resolve_deferred(id) {
            return true;
        }
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
            let enabled_features = self.supported_features();
            self.outbox.event(Event::Response(Response {
                id,
                sent: if sent { sent_info(message) } else { None },
                lines,
                history: None,
                history_targets: None,
                whois_channels,
                mode_list: None,
                state_limit: None,
                enabled_features,
            }));
        }
        true
    }

    pub(super) fn resolve_deferred(&mut self, id: RequestId) -> bool {
        let Some(index) = self.unlabelled_requests.iter().position(|request| {
            request.id == id
                && matches!(&request.matcher, ResponseMatcher::Join(channel)
                    if self.state.channel(channel).is_some_and(crate::Channel::is_synced))
        }) else {
            return false;
        };
        let request = self.unlabelled_requests.remove(index).unwrap();
        self.away_messages.remove(&id);
        let whois_channels = parse_whois_channels(&request.lines, self.isupport.value());
        let enabled_features = self.supported_features();
        self.outbox.event(Event::Response(Response {
            id,
            sent: None,
            lines: request.lines,
            history: None,
            history_targets: None,
            whois_channels,
            mode_list: None,
            state_limit: None,
            enabled_features,
        }));
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

    pub(super) fn resolve_unlabelled(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        labelled_request: Option<RequestId>,
    ) -> bool {
        let mapping = self.isupport.value().casemapping();
        let Some((index, complete, join_started)) = self
            .unlabelled_requests
            .iter()
            .enumerate()
            .find_map(|(index, request)| {
                if labelled_request.is_some_and(|id| id != request.id) {
                    return None;
                }
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
        let overflow = request.matcher.is_list_row(message, mapping)
            && request
                .lines
                .iter()
                .filter(|line| request.matcher.is_list_row(&line.as_ref(), mapping))
                .count()
                >= 1000;
        if overflow {
            let request = self.unlabelled_requests.remove(index).unwrap();
            self.requests
                .retain(|_, (pending, _, _, _, _)| *pending != request.id);
            if let ResponseMatcher::ListMode { channel, mode } = request.matcher {
                self.state.invalidate_mode_list(&channel, mode);
            }
            self.state
                .warn_limit(crate::StateLimit::ListEntries, &mut self.outbox);
            let enabled_features = self.supported_features();
            self.outbox.event(Event::Response(Response {
                id: request.id,
                sent: None,
                lines: request.lines,
                history: None,
                history_targets: None,
                whois_channels: Vec::new(),
                mode_list: None,
                state_limit: Some(crate::StateLimit::ListEntries),
                enabled_features,
            }));
            return true;
        }
        request.lines.push(message.to_owned());
        if complete {
            let request = self.unlabelled_requests.remove(index).unwrap();
            self.away_messages.remove(&request.id);
            let whois_channels = parse_whois_channels(&request.lines, self.isupport.value());
            let enabled_features = self.supported_features();
            let mut response = Response {
                id: request.id,
                sent: None,
                lines: request.lines,
                history: None,
                history_targets: None,
                whois_channels,
                mode_list: None,
                state_limit: None,
                enabled_features,
            };
            self.finish_mode_list_response(&request.matcher, &mut response);
            self.outbox.event(Event::Response(response));
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
        let labelled = self.caps.enabled().contains_key(&cap::LABELED_RESPONSE);
        let label = labelled.then(|| encode_label(self.next_request_id));
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        if let Some(label) = &label {
            message = message.with_tag("label", label);
        }
        let line = message.encode()?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if let Some(label) = label {
            self.requests
                .insert(label, (id, None, sent, feature, emit_events));
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox.transmit_tracked(line, None, Some(id));
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
