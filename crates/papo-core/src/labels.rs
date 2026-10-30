use crate::{
    Command, Event, Feature, Instant, Line, LineRef, MAX_PENDING_REQUESTS, Numeric, RequestError,
    RequestId, Response, ResponseMatcher, Session, UnlabelledRequest, cap, parse_whois_channels,
    sent_info,
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
        let whois_channels = parse_whois_channels(request.lines.as_slice(), self.isupport.value());
        let enabled_features = self.supported_features();
        self.outbox.event(Event::Response(Response {
            id,
            sent: None,
            lines: request.lines.into_vec(),
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
        let id = self.confirmed_away_request(message, true)?;
        self.away_messages.get(&id).map(String::as_str)
    }

    pub(super) fn confirmed_back(&self, message: &LineRef<'_>) -> bool {
        message.command() == Command::Numeric(Numeric::RPL_UNAWAY)
            && self.confirmed_away_request(message, false).is_some()
    }

    fn confirmed_away_request(&self, message: &LineRef<'_>, away: bool) -> Option<RequestId> {
        if let Some(label) = message.tag("label") {
            self.requests.get(label.as_ref()).map(|(id, ..)| *id)
        } else {
            self.unlabelled_requests.iter().find_map(|request| {
                matches!(&request.matcher, ResponseMatcher::Away(value) if *value == away)
                    .then_some(request.id)
            })
        }
    }

    pub(super) fn resolve_unlabelled(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        labelled_request: Option<RequestId>,
    ) -> bool {
        let mapping = self.isupport.value().casemapping();
        if self.abort_departed_join(message, mapping) {
            return true;
        }
        let Some((index, complete, join_started)) = self
            .unlabelled_requests
            .iter()
            .enumerate()
            .find_map(|(index, request)| {
                if request.deferred.is_some() || labelled_request.is_some_and(|id| id != request.id)
                {
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
                    .then(|| self.fence_completes(request, mapping))
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
        if request.lines.push_reject(message.to_owned()).is_err() {
            let mut request = self.unlabelled_requests.remove(index).unwrap();
            self.requests
                .retain(|_, (pending, _, _, _, _)| *pending != request.id);
            self.cancel_channel_join_matcher(&request.matcher);
            if let ResponseMatcher::ListMode { channel, mode } = request.matcher {
                self.state.invalidate_mode_list(&channel, mode);
            }
            if request.lines.take_overflow_warning() {
                self.state
                    .warn_limit(crate::StateLimit::ListEntries, &mut self.outbox);
            }
            let enabled_features = self.supported_features();
            self.outbox.event(Event::Response(Response {
                id: request.id,
                sent: None,
                lines: request.lines.into_vec(),
                history: None,
                history_targets: None,
                whois_channels: Vec::new(),
                mode_list: None,
                state_limit: Some(crate::StateLimit::ListEntries),
                enabled_features,
            }));
            return true;
        }
        if complete {
            let request = self.unlabelled_requests.remove(index).unwrap();
            self.away_messages.remove(&request.id);
            self.cancel_channel_join_matcher(&request.matcher);
            let whois_channels =
                parse_whois_channels(request.lines.as_slice(), self.isupport.value());
            let enabled_features = self.supported_features();
            let mut response = Response {
                id: request.id,
                sent: None,
                lines: request.lines.into_vec(),
                history: None,
                history_targets: None,
                whois_channels,
                mode_list: None,
                state_limit: None,
                enabled_features,
            };
            self.finish_mode_list_response(&request.matcher, &mut response);
            self.outbox.event(Event::Response(response));
            self.start_deferred_who();
        }
        true
    }

    pub(super) fn start_deferred_who(&mut self) {
        let Some(index) = self
            .unlabelled_requests
            .iter()
            .position(|request| request.deferred.is_some())
        else {
            return;
        };
        if self
            .unlabelled_requests
            .iter()
            .take(index)
            .any(|request| matches!(&request.matcher, ResponseMatcher::Who(_)))
        {
            return;
        }
        let request = &mut self.unlabelled_requests[index];
        let line = request.deferred.take().unwrap();
        let id = request.id;
        let fence = request.fence.clone();
        self.outbox.transmit_ordered(line, Some(id));
        if let Some(token) = fence {
            let ping = Line::new("PING")
                .param(token)
                .encode()
                .expect("internally generated PING is valid");
            self.outbox.transmit_ordered(ping, Some(id));
        }
    }

    fn fence_completes(&self, request: &UnlabelledRequest, mapping: crate::Casemapping) -> bool {
        !matches!(&request.matcher, ResponseMatcher::Who(_))
            || request.lines.iter().any(|line| {
                request
                    .matcher
                    .matches(
                        &line.as_ref(),
                        mapping,
                        self.welcome_state.nick(),
                        &self.state,
                    )
                    .is_some_and(|complete| complete)
            })
    }

    fn abort_departed_join(&mut self, message: &LineRef<'_>, mapping: crate::Casemapping) -> bool {
        let Some(index) =
            self.unlabelled_requests
                .iter()
                .position(|request| match &request.matcher {
                    ResponseMatcher::Join(channel) => ResponseMatcher::own_channel_departure(
                        message,
                        channel,
                        mapping,
                        self.welcome_state.nick(),
                        self.is_own_source(message),
                    ),
                    _ => false,
                })
        else {
            return false;
        };
        let request = self.unlabelled_requests.remove(index).unwrap();
        self.requests
            .retain(|_, (pending, _, _, _, _)| *pending != request.id);
        self.cancel_channel_join_matcher(&request.matcher);
        self.outbox.event(Event::RequestAborted {
            id: request.id,
            transmitted: request.started.is_some(),
        });
        true
    }
}

impl Session {
    pub(super) fn queue_request(
        &mut self,
        now: Instant,
        message: Line,
        feature: Option<Feature>,
        emit_events: bool,
    ) -> Result<RequestId, RequestError> {
        let id = RequestId(self.next_request_id);
        self.queue_request_with_id(now, message, feature, emit_events, id, None)?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        Ok(id)
    }

    pub(super) fn queue_reserved_request(
        &mut self,
        now: Instant,
        message: Line,
        feature: Feature,
        id: RequestId,
    ) -> Result<(), RequestError> {
        self.queue_request_with_id(now, message, Some(feature), true, id, None)
    }

    pub(super) fn queue_guarded_request(
        &mut self,
        now: Instant,
        message: Line,
        feature: Option<Feature>,
        emit_events: bool,
        guard: Option<crate::TargetGuard>,
    ) -> Result<RequestId, RequestError> {
        let id = RequestId(self.next_request_id);
        self.queue_request_with_id(now, message, feature, emit_events, id, guard)?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        Ok(id)
    }

    fn queue_request_with_id(
        &mut self,
        now: Instant,
        mut message: Line,
        feature: Option<Feature>,
        emit_events: bool,
        id: RequestId,
        guard: Option<crate::TargetGuard>,
    ) -> Result<(), RequestError> {
        self.rate_limit.advance(now);
        let sent = self.caps.enabled().contains_key(&cap::ECHO_MESSAGE)
            && matches!(
                message.as_ref().command(),
                Command::Privmsg | Command::Notice
            );
        let labelled = self.caps.enabled().contains_key(&cap::LABELED_RESPONSE);
        let label = labelled.then(|| encode_label(id.0));
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        if let Some(label) = &label {
            message = message.with_tag("label", label);
        }
        let line = message.encode()?;
        if let Some(label) = label {
            self.requests
                .insert(label, (id, None, sent, feature, emit_events));
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox.transmit_guarded(line, None, Some(id), guard);
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn back_confirmation_requires_the_numeric_and_matching_request() {
        let now = Instant::now();
        let mut session = Session::new(Config::default().nick("papo"));
        session
            .request_with_matcher(now, Line::new("AWAY"), ResponseMatcher::Away(false))
            .unwrap();

        for (wire, expected) in [
            (
                ":ergo.test 305 papo :You are no longer marked as being away",
                true,
            ),
            (
                ":ergo.test 306 papo :You have been marked as being away",
                false,
            ),
        ] {
            assert_eq!(
                session.confirmed_back(&LineRef::parse(wire).unwrap()),
                expected,
                "{wire}"
            );
        }

        let idle = Session::new(Config::default().nick("papo"));
        assert!(!idle.confirmed_back(
            &LineRef::parse(":ergo.test 305 papo :You are no longer marked as being away").unwrap()
        ));
    }
}
