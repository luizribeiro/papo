use crate::{
    Command, Event, Feature, Instant, Line, LineRef, MAX_PENDING_REQUESTS, Numeric, PendingRequest,
    RequestError, RequestId, Response, ResponseMatcher, SentEcho, Session, UnlabelledRequest, cap,
    parse_whois_channels, sent_info,
};

impl Session {
    pub(super) fn resolve_message(&mut self, message: &LineRef<'_>) -> Option<bool> {
        let label = message
            .tag("label")
            .map(std::borrow::Cow::into_owned)
            .or_else(|| self.pending_sent_label(message))?;
        if message.command() == Command::Numeric(Numeric::RPL_AWAY)
            && let Some(request) = self.requests.get_mut(&label)
            && request.sent
            && let Some(echo) = &mut request.echo
        {
            echo.reply = Some(message.to_owned());
            return Some(false);
        }
        let request = self.requests.remove(&label)?;
        let id = request.id;
        self.away_messages.remove(&id);
        if message.command_is("ACK") && self.resolve_deferred(id) {
            return Some(true);
        }
        let mut lines = request
            .echo
            .and_then(|echo| echo.reply)
            .into_iter()
            .collect::<Vec<_>>();
        if !message.command_is("ACK") {
            lines.push(message.to_owned());
        }
        let whois_channels = parse_whois_channels(&lines, self.isupport.value());
        if !self
            .unlabelled_requests
            .iter()
            .any(|request| request.id == id)
        {
            let enabled_features = self.supported_features();
            self.outbox.event(Event::Response(Response {
                id,
                sent: if request.sent {
                    sent_info(message)
                } else {
                    None
                },
                lines,
                history: None,
                history_targets: None,
                whois_channels,
                mode_list: None,
                state_limit: None,
                enabled_features,
            }));
        }
        Some(true)
    }

    fn pending_sent_label(&self, message: &LineRef<'_>) -> Option<String> {
        if message.tag("label").is_some() || !self.is_own_source(message) {
            return None;
        }
        let mapping = self.isupport.value().casemapping();
        self.requests
            .iter()
            .filter_map(|(label, request)| {
                let echo = request.echo.as_ref()?;
                (echo.reply.is_some() && echo.matches(message, mapping))
                    .then_some((request.id, label))
            })
            .min_by_key(|(id, _)| *id)
            .map(|(_, label)| label.clone())
    }

    pub(super) fn resolve_deferred(&mut self, id: RequestId) -> bool {
        let Some(index) = self.unlabelled_requests.iter().position(|request| {
            request.id == id
                && matches!(request.boundary.matcher(), Some(ResponseMatcher::Join(channel))
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
        let Some(complete) = self.resolve_message(message) else {
            return false;
        };
        if !emit_events {
            let response = complete.then(|| {
                self.outbox
                    .events
                    .pop_back()
                    .expect("a resolved request queued its response")
            });
            self.outbox.events.truncate(queued_events);
            if let Some(response) = response {
                self.outbox.event(response);
            }
        }
        true
    }

    pub(super) fn response_emits_events(&self, message: &LineRef<'_>) -> bool {
        let label = message
            .tag("label")
            .map(std::borrow::Cow::into_owned)
            .or_else(|| self.pending_sent_label(message));
        label
            .and_then(|label| self.requests.get(&label))
            .is_none_or(|request| request.emit_events)
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
            self.requests.get(label.as_ref()).map(|request| request.id)
        } else {
            self.unlabelled_requests.iter().find_map(|request| {
                matches!(request.boundary.matcher(), Some(ResponseMatcher::Away(value)) if *value == away)
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
                    request.boundary.matcher(),
                    Some(ResponseMatcher::Join(channel))
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
                            .boundary
                            .matches(message, mapping, self.welcome_state.nick(), &self.state)
                            .map(|complete| complete && request.fence.is_none())
                    })
                    .map(|done| (index, done, join_started))
            })
        else {
            return false;
        };
        let list_limit_reached = self.unlabelled_requests.get(index).is_some_and(|request| {
            request.boundary.is_list_row(message)
                && request
                    .lines
                    .iter()
                    .filter(|line| request.boundary.is_list_row(&line.as_ref()))
                    .count()
                    >= self.config.max_list_entries_per_mode
        });
        let request = self.unlabelled_requests.get_mut(index).unwrap();
        if join_started {
            request.started = Some(now);
        }
        if list_limit_reached || request.lines.push_reject(message.to_owned()).is_err() {
            let mut request = self.unlabelled_requests.remove(index).unwrap();
            self.requests.retain(|_, pending| pending.id != request.id);
            if let Some(ResponseMatcher::ListMode { channel, mode }) = request.boundary.matcher() {
                self.state.invalidate_mode_list(channel, *mode);
            }
            if list_limit_reached || request.lines.take_overflow_warning() {
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
            if let Some(matcher) = request.boundary.matcher() {
                self.finish_mode_list_response(matcher, &mut response);
            }
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
            .any(|request| matches!(request.boundary.matcher(), Some(ResponseMatcher::Who(_))))
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
        !matches!(request.boundary.matcher(), Some(ResponseMatcher::Who(_)))
            || request.lines.iter().any(|line| {
                request
                    .boundary
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
                .position(|request| match request.boundary.matcher() {
                    Some(ResponseMatcher::Join(channel)) => ResponseMatcher::own_channel_departure(
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
        self.requests.retain(|_, pending| pending.id != request.id);
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
        self.queue_request_with_id(now, message, feature, emit_events, id, None, true)?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        Ok(id)
    }

    pub(super) fn queue_registration_request(
        &mut self,
        now: Instant,
        message: Line,
        feature: Feature,
        id: RequestId,
    ) -> Result<(), RequestError> {
        self.queue_request_with_id(now, message, Some(feature), true, id, None, false)
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
        self.queue_request_with_id(now, message, feature, emit_events, id, guard, true)?;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_request_with_id(
        &mut self,
        now: Instant,
        mut message: Line,
        feature: Option<Feature>,
        emit_events: bool,
        id: RequestId,
        guard: Option<crate::TargetGuard>,
        requires_registration: bool,
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
            let echo = sent
                .then(|| SentEcho::from_message(&message.as_ref()))
                .flatten();
            self.requests.insert(
                label,
                PendingRequest {
                    id,
                    started: None,
                    sent,
                    feature,
                    emit_events,
                    echo,
                },
            );
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox
            .transmit_guarded(line, None, Some(id), guard, requires_registration);
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
