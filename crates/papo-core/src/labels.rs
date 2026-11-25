use crate::{
    Command, Event, Feature, IncomingLine, Instant, Line, LineRef, MAX_PENDING_REQUESTS, Numeric,
    PendingRequest, ReplyToken, RequestError, RequestId, Response, ResponseBoundary,
    ResponseMatcher, SentEcho, Session, UnlabelledRequest, WireLine, cap, parse_whois_channels,
    sent_info,
};

pub(super) fn who_target<'a>(message: &LineRef<'a>) -> Option<&'a str> {
    if message.command() != Command::Who
        || !matches!(message.param(1), None | Some(crate::state::WHOX_FIELDS))
    {
        return None;
    }
    message.param(0)
}

impl UnlabelledRequest {
    fn push_or_overflow(
        &mut self,
        message: Line,
        complete: bool,
        max_list_entries: usize,
        subject: Option<ReplyToken>,
    ) -> bool {
        if self.boundary.is_framing(&message.as_ref()) {
            return false;
        }
        let index = self.lines.as_slice().len();
        let overflow = self.list_rows_full(&message.as_ref(), max_list_entries)
            || self
                .lines
                .push_reject_unless_last(message, complete)
                .is_err();
        if !overflow && let Some(token) = subject {
            self.subject_tokens.push((index, token));
        }
        overflow
    }

    fn list_rows_full(&self, message: &LineRef<'_>, maximum: usize) -> bool {
        self.boundary.is_list_row(message)
            && self
                .lines
                .iter()
                .filter(|line| self.boundary.is_list_row(&line.as_ref()))
                .count()
                >= maximum
    }
}

impl Session {
    pub(super) fn who_reply_target(
        &self,
        message: &LineRef<'_>,
        enclosing_request: Option<(RequestId, Option<&str>)>,
    ) -> Option<String> {
        if !matches!(
            message.command(),
            Command::Numeric(Numeric::RPL_WHOREPLY | Numeric::RPL_WHOSPCRPL)
        ) {
            return None;
        }
        if let Some((_, target)) = enclosing_request {
            return target.map(str::to_owned);
        }
        if let Some(label) = message.tag("label") {
            return self.requests.get(label.as_ref())?.who_target.clone();
        }
        self.unlabelled_requests.iter().find_map(|request| {
            request.deferred.is_none().then_some(())?;
            request.who_target.clone()
        })
    }

    pub(super) fn reply_line(&self, incoming: IncomingLine<'_>, request: &PendingRequest) -> Line {
        let routing = incoming.routing();
        let mut addresses = Vec::new();
        if let Some(matcher) = &request.matcher
            && matcher
                .matches(
                    &routing,
                    self.isupport.value().casemapping(),
                    self.welcome_state.nick(),
                    &self.state,
                    &mut addresses,
                )
                .is_none()
        {
            addresses.clear();
        }
        incoming.addressed(&addresses)
    }

    /// Classifies a standard failure by the command and code the server sent.
    pub(super) fn standard_refusal(&self, incoming: IncomingLine<'_>) -> Option<crate::Refusal> {
        let routing = incoming.routing();
        let reply = routing.standard_reply()?;
        (reply.kind == papo_proto::StandardReplyKind::Fail)
            .then(|| reply.refusal(&self.supported_features()))
    }

    pub(super) fn batch_reply_lines(
        &self,
        messages: Vec<crate::StoredLine>,
        request: &PendingRequest,
    ) -> Vec<Line> {
        messages
            .into_iter()
            .map(|line| self.reply_line(line.incoming(), request))
            .collect()
    }

    pub(super) fn resolve_message(&mut self, incoming: IncomingLine<'_>) -> Option<bool> {
        let message = &incoming.shown;
        let label = message
            .tag("label")
            .map(std::borrow::Cow::into_owned)
            .or_else(|| self.pending_sent_label(message))?;
        if let Some(request) = self.requests.get_mut(&label)
            && request.sent
            && let Some(echo) = &mut request.echo
            && message.command() == Command::Numeric(Numeric::RPL_AWAY)
        {
            echo.reply = Some(incoming.shown.to_owned());
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
            .as_ref()
            .and_then(|echo| echo.reply.clone())
            .into_iter()
            .collect::<Vec<_>>();
        if !message.command_is("ACK") {
            lines.push(self.reply_line(incoming, &request));
        }
        if !self
            .unlabelled_requests
            .iter()
            .any(|request| request.id == id)
        {
            let echo = if message.command_is("ACK") {
                lines.first().map(Line::as_ref)
            } else {
                Some(incoming.shown)
            };
            let sent = echo
                .filter(|echo| request.sent && is_echoed(echo.command()))
                .and_then(|echo| sent_info(&echo));
            let response = Response {
                sent,
                standard_refusal: self.standard_refusal(incoming),
                ..self.completed_response(id, lines)
            };
            self.outbox.event(Event::Response(response));
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
                // A pending REDACT ends only at its labelled reply, so a repeated echo of it
                // cannot answer it.
                (echo.command != Command::Redact
                    && echo.reply.is_some()
                    && echo.matches(message, mapping))
                .then_some((request.id, label))
            })
            .min_by_key(|(id, _)| *id)
            .map(|(_, label)| label.clone())
    }

    /// Builds the response to request `id` from its delivered `lines`; every optional result
    /// starts absent.
    pub(super) fn completed_response(&self, id: RequestId, lines: Vec<Line>) -> Response {
        Response {
            id,
            sent: None,
            whois_channels: parse_whois_channels(&lines, self.isupport.value()),
            lines,
            history: None,
            history_targets: None,
            mode_list: None,
            state_limit: None,
            enabled_features: self.supported_features(),
            subject_tokens: Vec::new(),
            standard_refusal: None,
        }
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
        let response = self.completed_response(id, request.lines.into_vec());
        self.outbox.event(Event::Response(response));
        true
    }

    pub(super) fn resolve_labelled_message(
        &mut self,
        incoming: IncomingLine<'_>,
        queued_events: usize,
        emit_events: bool,
    ) -> bool {
        let Some(complete) = self.resolve_message(incoming) else {
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

    /// Whether `message`, inside the labelled batch answering `request`, is part of that answer.
    ///
    /// A send is answered by its own echo and its failure. Anything else in its batch, such as
    /// a service's reply or a mode change the send caused, is traffic the server would have sent
    /// unbatched. Every line answers any other request.
    pub(super) fn answers_request(&self, request: &PendingRequest, message: &LineRef<'_>) -> bool {
        !request.sent || self.is_own_echo(message) || crate::is_failure(message)
    }

    pub(super) fn response_emits_events(&self, message: &LineRef<'_>) -> bool {
        let label = message
            .tag("label")
            .map(std::borrow::Cow::into_owned)
            .or_else(|| self.pending_sent_label(message));
        label
            .and_then(|label| self.requests.get(&label))
            .is_none_or(|request| {
                // A send's result carries echo identity. Its away reply is presence information.
                request.emit_events
                    || (request.sent && message.command() == Command::Numeric(Numeric::RPL_AWAY))
            })
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
        incoming: IncomingLine<'_>,
        labelled_request: Option<RequestId>,
        joined_ready: Option<bool>,
    ) -> bool {
        let routing = incoming.routing();
        let message = &incoming.shown;
        let mapping = self.isupport.value().casemapping();
        if self.abort_departed_join(incoming, mapping) {
            return true;
        }
        let Some((index, complete, join_started, reached_boundary, addresses)) = self
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
                    .then(|| {
                        !matches!(request.boundary.matcher(), Some(ResponseMatcher::Who(_)))
                            || request.boundary_seen
                    })
                });
                let mut addresses = Vec::new();
                let matched = fence.or_else(|| {
                    request
                        .boundary
                        .matches(
                            &routing,
                            mapping,
                            self.welcome_state.nick(),
                            &self.state,
                            &mut addresses,
                        )
                        .map(|complete| {
                            if join_started {
                                joined_ready.unwrap_or(complete)
                            } else {
                                complete
                            }
                        })
                })?;
                let complete = matched && (fence.is_some() || request.fence.is_none());
                Some((index, complete, join_started, matched, addresses))
            })
        else {
            return false;
        };
        let shown = incoming.addressed(&addresses);
        let standard_refusal = self.standard_refusal(incoming);
        let max_list_entries = self.config.max_list_entries_per_mode;
        let subject = self.state.who_subject_token(message);
        let request = self.unlabelled_requests.get_mut(index).unwrap();
        if join_started {
            request.started = Some(now);
        }
        request.boundary_seen |= reached_boundary;
        request.standard_refusal = request.standard_refusal.or(standard_refusal);
        if request.push_or_overflow(shown, complete, max_list_entries, subject) {
            let id = request.id;
            self.finish_truncated(id);
            return true;
        }
        if complete {
            let request = self.unlabelled_requests.remove(index).unwrap();
            self.away_messages.remove(&request.id);
            let mut response = Response {
                subject_tokens: request.subject_tokens,
                standard_refusal: request.standard_refusal,
                ..self.completed_response(request.id, request.lines.into_vec())
            };
            if let Some(matcher) = request.boundary.matcher() {
                self.finish_mode_list_response(matcher, &mut response);
            }
            self.outbox.event(Event::Response(response));
            self.start_deferred_who();
        }
        true
    }

    pub(super) fn finish_truncated(&mut self, id: RequestId) {
        self.requests.retain(|_, pending| pending.id != id);
        let lines = match self
            .unlabelled_requests
            .iter()
            .position(|request| request.id == id)
        {
            Some(index) => {
                let request = self.unlabelled_requests.remove(index).unwrap();
                if let Some(ResponseMatcher::ListMode { channel, mode }) =
                    request.boundary.matcher()
                {
                    self.state.invalidate_mode_list(channel, *mode);
                }
                request.lines.into_vec()
            }
            None => Vec::new(),
        };
        self.state
            .warn_limit(crate::StateLimit::ListEntries, &mut self.outbox);
        let response = Response {
            whois_channels: Vec::new(),
            state_limit: Some(crate::StateLimit::ListEntries),
            ..self.completed_response(id, lines)
        };
        self.outbox.event(Event::Response(response));
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

    fn abort_departed_join(
        &mut self,
        incoming: IncomingLine<'_>,
        mapping: crate::Casemapping,
    ) -> bool {
        let routing = incoming.routing();
        let Some(index) =
            self.unlabelled_requests
                .iter()
                .position(|request| match request.boundary.matcher() {
                    Some(ResponseMatcher::Join(channel)) => ResponseMatcher::own_channel_departure(
                        &routing,
                        channel,
                        mapping,
                        self.welcome_state.nick(),
                        self.is_own_source(&incoming.shown),
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
    pub(super) fn track_unlabelled(
        &mut self,
        id: RequestId,
        boundary: ResponseBoundary,
        row_limit: usize,
        fence: Option<String>,
        deferred: Option<WireLine>,
        who_target: Option<String>,
    ) {
        let channel_join = boundary
            .matcher()
            .and_then(|matcher| self.channel_join_reservation(matcher));
        self.unlabelled_requests.push_back(UnlabelledRequest {
            id,
            started: None,
            boundary,
            channel_join,
            fence,
            deferred,
            who_target,
            lines: crate::bounded::BoundedCollection::new(row_limit),
            boundary_seen: false,
            subject_tokens: Vec::new(),
            standard_refusal: None,
        });
    }

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
    pub(super) fn queue_request_with_id(
        &mut self,
        now: Instant,
        mut message: Line,
        feature: Option<Feature>,
        emit_events: bool,
        id: RequestId,
        guard: Option<crate::TargetGuard>,
        requires_registration: bool,
    ) -> Result<(), RequestError> {
        self.check_line_audience(&message.as_ref())?;
        self.rate_limit.advance(now);
        let sent = self.echoes(&message.as_ref());
        let labelled = self.caps.enabled().contains_key(&cap::LABELED_RESPONSE);
        let label = labelled.then(|| encode_label(id.0));
        if labelled && self.requests.len() >= MAX_PENDING_REQUESTS {
            return Err(RequestError::PendingLimit);
        }
        if let Some(label) = &label {
            message = message.with_tag("label", label);
        }
        let line = self.encode_user_line(now, &message)?;
        if let Some(label) = label {
            let echo = sent.then(|| SentEcho::from_message(&message)).flatten();
            self.requests.insert(
                label,
                PendingRequest {
                    id,
                    started: None,
                    sent,
                    feature,
                    emit_events,
                    echo,
                    who_target: None,
                    matcher: None,
                },
            );
        } else {
            self.outbox.event(Event::RequestUntracked { id });
        }
        self.outbox
            .transmit_guarded(line, None, Some(id), guard, requires_registration);
        Ok(())
    }

    /// Whether the server will echo `message` back to us.
    ///
    /// Ergo relays our `REDACT` to us as to every other member, with or without `echo-message`.
    fn echoes(&self, message: &LineRef<'_>) -> bool {
        message.command() == Command::Redact
            || self.caps.enabled().contains_key(&cap::ECHO_MESSAGE)
                && (is_echoed(message.command()) || message.command_is("RELAYMSG"))
    }

    /// Encodes a line we send, tracking the secrets its echo will repeat.
    pub(super) fn encode_user_line(
        &mut self,
        now: Instant,
        message: &Line,
    ) -> Result<crate::WireLine, RequestError> {
        let line = message.encode()?;
        if self.echoes(&message.as_ref()) {
            self.secret_echoes.track(now, message)?;
        }
        Ok(line)
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

pub(super) fn is_echoed(command: Command) -> bool {
    matches!(
        command,
        Command::Privmsg | Command::Notice | Command::Tagmsg | Command::Redact
    )
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
