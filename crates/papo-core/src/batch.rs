use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};
use web_time::Instant;

use papo_proto::{Command, Line, LineRef, Reaction};

use crate::secret_echo::{IncomingLine, StoredLine};
use crate::{BatchAbandonReason, Config, HistoryItem, HistoryLine, ReplyToken, SplitServers, cap};

const MAX_ABANDONED_BATCHES: usize = 256;
pub(crate) const DEFAULT_BYTE_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct Batches {
    open: BTreeMap<String, OpenBatch>,
    abandoned: BTreeMap<String, AbandonedBatch>,
    pub(crate) provenance_exhausted: bool,
    max_open: usize,
    max_lines: usize,
    byte_budget: usize,
    timeout: Duration,
    buffered_bytes: usize,
}

#[derive(Debug)]
struct AbandonedBatch {
    kind: String,
    parent: Option<String>,
    reason: BatchAbandonReason,
    replay: bool,
}

#[derive(Debug)]
struct OpenBatch {
    opening: Line,
    kind: String,
    label: Option<String>,
    parent: Option<String>,
    params: Vec<String>,
    exhausted: bool,
    messages: Vec<StoredLine>,
    incomplete: Vec<usize>,
    routed_members: RoutedMembers,
    completed: Vec<BatchEvent>,
    multiline_collectors: Vec<String>,
    multiline_sender: Option<ReplyToken>,
    lines: usize,
    multiline_bytes: usize,
    buffered_bytes: usize,
    opened: Instant,
    typed_aggregate: bool,
    replay: bool,
}

pub(crate) type RoutedMembers = BTreeMap<usize, RoutedMember>;

#[derive(Clone, Copy, Debug)]
pub(crate) struct RoutedMember {
    pub(crate) live: bool,
    pub(crate) replay_copy: bool,
    pub(crate) subject: Option<ReplyToken>,
    pub(crate) joined_ready: Option<bool>,
    pub(crate) sender: Option<ReplyToken>,
}

impl RoutedMember {
    fn replay(copy: bool) -> Self {
        Self {
            live: false,
            replay_copy: copy,
            subject: None,
            joined_ready: None,
            sender: None,
        }
    }

    fn observed(sender: ReplyToken) -> Self {
        Self {
            live: false,
            replay_copy: false,
            subject: None,
            joined_ready: None,
            sender: Some(sender),
        }
    }
}

pub(crate) struct LiveMember<'a> {
    pub(crate) kind: &'a str,
    pub(crate) label: Option<&'a str>,
    pub(crate) split: Option<SplitServers>,
    pub(crate) raw: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DispatchContext {
    batch_type: Option<String>,
    pub(crate) defer_responses: bool,
    pub(crate) live_input: bool,
    pub(crate) sender: Option<ReplyToken>,
}

impl DispatchContext {
    pub(crate) fn batch_type(&self) -> Option<&str> {
        self.batch_type.as_deref()
    }

    pub(crate) fn from_batch_type(batch_type: String) -> Self {
        Self {
            batch_type: Some(batch_type),
            ..Self::default()
        }
    }
}

pub(crate) enum BatchAction {
    Consumed,
    Complete(Vec<BatchEvent>),
    Dispatch(DispatchContext),
}

#[derive(Debug)]
pub(crate) enum BatchEvent {
    Response {
        label: String,
        echo: Option<Line>,
        messages: Vec<StoredLine>,
        batch_type: String,
        routed_members: RoutedMembers,
        replay: bool,
        history: Option<(String, Vec<HistoryItem>, bool)>,
        history_targets: Option<Vec<Line>>,
    },
    History {
        target: String,
        items: Vec<HistoryItem>,
        exhausted: bool,
    },
    HistoryTargets(Vec<Line>),
    Abandoned {
        reference: String,
        batch_type: String,
        reason: BatchAbandonReason,
        label: Option<String>,
    },
    Line {
        line: Line,
        batch_type: Option<String>,
        sender: Option<ReplyToken>,
    },
    Multiline(Line, Option<ReplyToken>),
    Incomplete(Line, Option<ReplyToken>),
    Metadata {
        kind: String,
        messages: Vec<StoredLine>,
    },
    #[cfg(feature = "unstable")]
    Bouncer(Vec<StoredLine>),
    Isupport(Vec<StoredLine>),
}

impl Batches {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            open: BTreeMap::new(),
            abandoned: BTreeMap::new(),
            provenance_exhausted: false,
            max_open: config.max_open_batches,
            max_lines: config.max_batch_lines,
            byte_budget: config.batch_byte_budget,
            timeout: config.batch_timeout,
            buffered_bytes: 0,
        }
    }

    pub(crate) fn reset(&mut self) -> Vec<BatchEvent> {
        let mut events: Vec<_> = self
            .open
            .values()
            .filter(|batch| !batch.replay && batch.kind == cap::MULTILINE.as_str())
            .filter_map(|batch| {
                multiline(batch).map(|line| BatchEvent::Incomplete(line, batch.multiline_sender))
            })
            .collect();
        let roots: Vec<_> = self
            .open
            .iter()
            .filter(|(_, batch)| batch.parent.is_none())
            .map(|(reference, _)| reference.clone())
            .collect();
        for root in roots {
            events.extend(self.drain(&root, None).into_iter().filter(|event| {
                matches!(event, BatchEvent::History { .. })
                    || matches!(event, BatchEvent::Line { batch_type: Some(kind), .. }
                        if is_replay(kind))
            }));
        }
        self.abandoned.clear();
        self.provenance_exhausted = false;
        self.buffered_bytes = 0;
        events
    }

    pub(crate) fn handle<'a>(
        &mut self,
        now: Instant,
        incoming: impl Into<IncomingLine<'a>>,
        multiline_max_bytes: usize,
        bouncer_enabled: bool,
        correlated_label: bool,
        applied_state: Option<RoutedMember>,
    ) -> BatchAction {
        let incoming = incoming.into();
        let routing = incoming.routing();
        let message = &routing;
        if !message.command_is("BATCH") {
            return self.member(incoming, multiline_max_bytes, applied_state);
        }
        let Some(reference) = message.param(0) else {
            return BatchAction::Dispatch(self.context(message));
        };
        if let Some(reference) = reference.strip_prefix('+') {
            return self.open(now, message, reference, bouncer_enabled, correlated_label);
        }
        if let Some(reference) = reference.strip_prefix('-') {
            if let Some(batch) = self.abandoned.get(reference) {
                let kind = batch
                    .replay
                    .then(|| dispatch_kind(&batch.kind, true).to_owned());
                self.close_abandoned(message, reference);
                if let Some(kind) = kind {
                    return BatchAction::Complete(vec![BatchEvent::Line {
                        line: incoming.shown.to_owned(),
                        batch_type: Some(kind),
                        sender: None,
                    }]);
                }
            } else {
                return self.close(message, reference);
            }
        }
        BatchAction::Dispatch(self.context(message))
    }

    fn open(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        reference: &str,
        bouncer_enabled: bool,
        correlated_label: bool,
    ) -> BatchAction {
        let Some(kind) = message.param(1) else {
            return BatchAction::Dispatch(self.context(message));
        };
        let parent = message.tag("batch").map(std::borrow::Cow::into_owned);
        let open_parent = parent.as_ref().and_then(|parent| self.open.get(parent));
        let abandoned_parent = parent
            .as_ref()
            .and_then(|parent| self.abandoned.get(parent));
        if !valid_reference(reference)
            || self.open.contains_key(reference)
            || self.abandoned.contains_key(reference)
            || (parent.is_some() && open_parent.is_none() && abandoned_parent.is_none())
        {
            return BatchAction::Dispatch(self.context(message));
        }
        let replay_parent = open_parent.is_some_and(|parent| parent.replay)
            || abandoned_parent.is_some_and(|parent| parent.replay);
        let replay = replay_parent || is_replay(kind);
        let label = (correlated_label && !replay_parent)
            .then(|| message.tag("label"))
            .flatten()
            .map(std::borrow::Cow::into_owned);
        let abandon = abandoned_parent.map(|parent| parent.reason).or_else(|| {
            (self.open.len() >= self.max_open).then_some(BatchAbandonReason::OpenBatchLimit)
        });
        if let Some(reason) = abandon {
            let label = label.or_else(|| self.enclosing_history_label(kind, parent.as_deref()));
            self.remember_abandoned(
                reference,
                AbandonedBatch {
                    kind: kind.to_owned(),
                    parent,
                    reason,
                    replay,
                },
            );
            return BatchAction::Complete(vec![BatchEvent::Abandoned {
                reference: reference.to_owned(),
                batch_type: kind.to_owned(),
                reason,
                label,
            }]);
        }
        let multiline_collectors = if kind == cap::MULTILINE.as_str() {
            parent
                .as_deref()
                .filter(|_| replay == replay_parent)
                .map(|parent| self.aggregate_references(parent))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.open.insert(
            reference.to_owned(),
            OpenBatch {
                opening: message.to_owned(),
                kind: kind.to_owned(),
                label,
                parent,
                params: message.params().skip(2).map(str::to_owned).collect(),
                exhausted: message
                    .tags()
                    .any(|(key, _)| key == "draft/chathistory-end"),
                messages: Vec::new(),
                incomplete: Vec::new(),
                routed_members: BTreeMap::new(),
                completed: Vec::new(),
                multiline_collectors: multiline_collectors.clone(),
                multiline_sender: None,
                lines: 0,
                multiline_bytes: 0,
                buffered_bytes: retained_bytes(message),
                opened: now,
                typed_aggregate: if replay {
                    is_history(kind) || kind == cap::MULTILINE.as_str()
                } else {
                    is_aggregate(kind, bouncer_enabled)
                },
                replay,
            },
        );
        self.buffered_bytes = self.buffered_bytes.saturating_add(retained_bytes(message));
        for (index, collector) in multiline_collectors.into_iter().enumerate() {
            let batch = self
                .open
                .get_mut(&collector)
                .expect("resolved multiline collector remains open");
            if replay && !batch.replay {
                batch
                    .routed_members
                    .insert(batch.messages.len(), RoutedMember::replay(index != 0));
            }
            batch.messages.push(message.to_owned().into());
            batch.buffered_bytes = batch.buffered_bytes.saturating_add(retained_bytes(message));
            self.buffered_bytes = self.buffered_bytes.saturating_add(retained_bytes(message));
        }
        BatchAction::Consumed
    }

    fn close_abandoned(&mut self, message: &LineRef<'_>, reference: &str) {
        let parent = message.tag("batch");
        if message.len() == 1
            && self.abandoned.get(reference).is_some_and(|batch| {
                parent.is_none() || parent.as_deref() == batch.parent.as_deref()
            })
            && !self
                .abandoned
                .values()
                .any(|batch| batch.parent.as_deref() == Some(reference))
        {
            self.abandoned.remove(reference);
        }
    }

    fn close(&mut self, message: &LineRef<'_>, reference: &str) -> BatchAction {
        let actual_parent = message.tag("batch");
        let valid = valid_reference(reference)
            && message.len() == 1
            && self.open.get(reference).is_some_and(|batch| {
                (actual_parent.is_none() || batch.parent.as_deref() == actual_parent.as_deref())
                    && !self
                        .open
                        .values()
                        .any(|candidate| candidate.parent.as_deref() == Some(reference))
            });
        if !valid {
            return BatchAction::Dispatch(self.context(message));
        }
        let mut batch = self.open.remove(reference).expect("validated above");
        let moved_bytes = batch.buffered_bytes;
        self.buffered_bytes = self.buffered_bytes.saturating_sub(batch.buffered_bytes);
        let mut completed = std::mem::take(&mut batch.completed);
        let response_messages = batch.label.as_ref().map(|_| batch.messages.clone());
        if batch.kind != cap::MULTILINE.as_str() && !batch.routed_members.is_empty() {
            batch.messages = unapplied_messages(batch.messages, &batch.routed_members).collect();
        }
        if batch.is_typed_aggregate()
            && (batch.kind == cap::MULTILINE.as_str()
                || !self.has_typed_aggregate_ancestor(batch.parent.as_deref(), batch.replay))
        {
            completed.extend(complete(&mut batch));
        }
        if !batch.multiline_collectors.is_empty() {
            let assembled = completed.iter().find_map(|event| match event {
                BatchEvent::Multiline(message, _) => Some(message.clone()),
                _ => None,
            });
            let sender = assembled.as_ref().and(batch.multiline_sender);
            let replacement = multiline_rows(&batch, assembled);
            self.replace_multiline_placeholders(
                reference,
                &batch.multiline_collectors,
                &replacement,
                false,
                sender,
            );
        }
        if let Some(label) = batch.label.take() {
            let echo = completed.iter().find_map(|event| match event {
                BatchEvent::Multiline(message, _) => Some(message.clone()),
                _ => None,
            });
            let history = completed
                .iter()
                .position(|event| matches!(event, BatchEvent::History { .. }))
                .and_then(|index| history_from_event(completed.remove(index)));
            let history_targets = completed
                .iter()
                .position(|event| matches!(event, BatchEvent::HistoryTargets(_)))
                .and_then(|index| match completed.remove(index) {
                    BatchEvent::HistoryTargets(rows) => Some(rows),
                    _ => None,
                });
            completed.push(BatchEvent::Response {
                label,
                echo,
                messages: response_messages.expect("label captured response messages"),
                batch_type: batch.kind.clone(),
                routed_members: batch.routed_members,
                replay: batch.replay,
                history,
                history_targets,
            });
        }
        if !batch.multiline_collectors.is_empty() {
            completed.retain(|event| !matches!(event, BatchEvent::Multiline(..)));
        }
        if let Some(parent) = batch.parent {
            let retained = moved_bytes.saturating_add(
                completed
                    .len()
                    .saturating_mul(std::mem::size_of::<BatchEvent>()),
            );
            let parent_batch = self
                .open
                .get_mut(&parent)
                .expect("validated batch parent remains open");
            parent_batch.lines = parent_batch.lines.saturating_add(completed.len());
            parent_batch.buffered_bytes = parent_batch.buffered_bytes.saturating_add(retained);
            parent_batch.completed.append(&mut completed);
            self.buffered_bytes = self.buffered_bytes.saturating_add(retained);
            if parent_batch.lines > self.max_lines || self.buffered_bytes > self.byte_budget {
                let reason = if parent_batch.lines > self.max_lines {
                    BatchAbandonReason::LineLimit
                } else {
                    BatchAbandonReason::ByteLimit
                };
                BatchAction::Complete(self.abandon(&parent, reason))
            } else {
                BatchAction::Consumed
            }
        } else {
            BatchAction::Complete(completed)
        }
    }

    #[allow(clippy::too_many_lines)]
    fn member(
        &mut self,
        incoming: IncomingLine<'_>,
        multiline_max_bytes: usize,
        applied_state: Option<RoutedMember>,
    ) -> BatchAction {
        let routing = incoming.routing();
        let message = &routing;
        let context = self.context(message);
        let Some(reference) = message.tag("batch") else {
            return BatchAction::Dispatch(context);
        };
        if !self.open.contains_key(reference.as_ref()) {
            return BatchAction::Dispatch(if context.batch_type().is_some_and(is_replay) {
                context
            } else {
                DispatchContext {
                    live_input: true,
                    ..DispatchContext::default()
                }
            });
        }
        let ancestors = self.ancestors(reference.as_ref());
        let retained = retained_incoming(incoming);
        for reference in &ancestors {
            let batch = self.open.get_mut(reference).expect("resolved ancestor");
            batch.lines = batch.lines.saturating_add(1);
            batch.buffered_bytes = batch.buffered_bytes.saturating_add(retained);
            self.buffered_bytes = self.buffered_bytes.saturating_add(retained);
            if batch.kind == cap::MULTILINE.as_str() {
                let concat = message
                    .tags()
                    .any(|(key, _)| key == "draft/multiline-concat");
                batch.multiline_bytes = batch.multiline_bytes.saturating_add(
                    message.param(1).map_or(0, str::len)
                        + usize::from(!batch.messages.is_empty() && !concat),
                );
            }
        }
        let aggregates = self.aggregate_references(reference.as_ref());
        let replay = self.open[reference.as_ref()].replay;
        // The first collector owns replay delivery. Later labeled collectors keep raw copies
        // for the response, even if the owning batch closes or is abandoned before the wrapper.
        for (index, reference) in aggregates.iter().enumerate() {
            let batch = self
                .open
                .get_mut(reference)
                .expect("resolved aggregate remains open");
            let route = if replay && !batch.replay {
                Some(RoutedMember::replay(index != 0))
            } else {
                applied_state
            };
            if let Some(route) = route {
                if batch.label.is_some() {
                    let overhead = std::mem::size_of::<(usize, RoutedMember)>();
                    batch.buffered_bytes = batch.buffered_bytes.saturating_add(overhead);
                    self.buffered_bytes = self.buffered_bytes.saturating_add(overhead);
                    batch.routed_members.insert(batch.messages.len(), route);
                } else if batch.kind != cap::MULTILINE.as_str() {
                    continue;
                }
            }
            batch.messages.push(incoming.store());
        }
        if let Some(reference) = ancestors
            .iter()
            .rev()
            .find(|reference| {
                self.open[*reference].lines > self.max_lines
                    || (self.open[*reference].kind == cap::MULTILINE.as_str()
                        && self.open[*reference].multiline_bytes > multiline_max_bytes)
                    || self.buffered_bytes > self.byte_budget
            })
            .cloned()
        {
            let reason = if self.open[&reference].lines > self.max_lines {
                BatchAbandonReason::LineLimit
            } else {
                BatchAbandonReason::ByteLimit
            };
            let mut events = self.abandon(&reference, reason);
            if aggregates.is_empty() && applied_state.is_none() {
                let fallback = BatchEvent::Line {
                    line: incoming.shown.to_owned(),
                    batch_type: context.batch_type,
                    sender: None,
                };
                events.insert(events.len() - 1, fallback);
            }
            return BatchAction::Complete(events);
        }
        if applied_state.is_some() || !aggregates.is_empty() {
            BatchAction::Consumed
        } else {
            BatchAction::Complete(vec![BatchEvent::Line {
                line: incoming.shown.to_owned(),
                batch_type: context.batch_type,
                sender: None,
            }])
        }
    }

    fn ancestors(&self, reference: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = Some(reference);
        while let Some(reference) = current {
            let Some(batch) = self.open.get(reference) else {
                break;
            };
            result.push(reference.to_owned());
            current = batch.parent.as_deref();
        }
        result
    }

    pub(crate) fn handle_timeout(&mut self, now: Instant) -> Vec<BatchEvent> {
        let expired: Vec<_> = self
            .open
            .iter()
            .filter(|(_, batch)| {
                crate::deadline(batch.opened, self.timeout).is_some_and(|deadline| now >= deadline)
            })
            .map(|(reference, _)| reference.clone())
            .collect();
        let roots: Vec<_> = expired
            .iter()
            .filter(|reference| {
                self.open[*reference]
                    .parent
                    .as_ref()
                    .is_none_or(|parent| !expired.contains(parent))
            })
            .cloned()
            .collect();
        roots
            .into_iter()
            .flat_map(|reference| self.abandon(&reference, BatchAbandonReason::Timeout))
            .collect()
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.open
            .values()
            .filter_map(|batch| crate::deadline(batch.opened, self.timeout))
            .min()
    }

    fn incomplete_multilines(&self, references: &[String]) -> BTreeMap<String, Line> {
        references
            .iter()
            .filter_map(|candidate| {
                let batch = &self.open[candidate];
                (batch.kind == cap::MULTILINE.as_str())
                    .then(|| multiline(batch).map(|line| (candidate.clone(), line)))
                    .flatten()
            })
            .collect()
    }

    fn abandon(&mut self, reference: &str, reason: BatchAbandonReason) -> Vec<BatchEvent> {
        let batch = &self.open[reference];
        let warning = BatchEvent::Abandoned {
            reference: reference.to_owned(),
            batch_type: batch.kind.clone(),
            reason,
            label: batch
                .label
                .clone()
                .or_else(|| self.enclosing_history_label(&batch.kind, batch.parent.as_deref())),
        };
        let mut events = self.drain(reference, Some(reason));
        events.push(warning);
        events
    }

    fn enclosing_history_label<'a>(
        &'a self,
        kind: &str,
        mut parent: Option<&'a str>,
    ) -> Option<String> {
        if !is_history(kind) {
            return None;
        }
        while let Some(batch) = parent.and_then(|reference| self.open.get(reference)) {
            if batch.kind == cap::LABELED_RESPONSE.as_str() && batch.label.is_some() {
                return batch.label.clone();
            }
            parent = batch.parent.as_deref();
        }
        None
    }

    #[allow(clippy::too_many_lines)]
    fn drain(&mut self, reference: &str, reason: Option<BatchAbandonReason>) -> Vec<BatchEvent> {
        let references: Vec<_> = self
            .open
            .keys()
            .filter(|candidate| {
                self.ancestors(candidate)
                    .iter()
                    .any(|item| item == reference)
            })
            .cloned()
            .collect();
        let mut types = BTreeMap::new();
        for candidate in &references {
            let batch = &self.open[candidate];
            types.insert(
                candidate.clone(),
                dispatch_kind(&batch.kind, batch.replay).to_owned(),
            );
        }
        let batch = &self.open[reference];
        let history_target = is_history(&batch.kind)
            .then(|| batch.params.first().cloned())
            .flatten();
        let incomplete = self.incomplete_multilines(&references);
        let absorbed_multiline: HashSet<_> = references
            .iter()
            .filter_map(|candidate| {
                let collectors = self.open[candidate].multiline_collectors.clone();
                if collectors.is_empty() {
                    return None;
                }
                let line = incomplete.get(candidate);
                let batch = &self.open[candidate];
                let sender = line.and(batch.multiline_sender);
                let replacement = multiline_rows(batch, line.cloned());
                self.replace_multiline_placeholders(
                    candidate,
                    &collectors,
                    &replacement,
                    line.is_some(),
                    sender,
                );
                Some(candidate.clone())
            })
            .collect();
        let mut messages = Vec::new();
        let mut history_incomplete = Vec::new();
        let mut incomplete_events = Vec::new();
        for candidate in references {
            let batch = self.open.remove(&candidate).expect("selected open batch");
            if let Some(reason) = reason {
                self.remember_abandoned(&candidate, batch.abandoned(reason));
            }
            self.buffered_bytes = self.buffered_bytes.saturating_sub(batch.buffered_bytes);
            history_incomplete.extend(batch.incomplete.iter().filter_map(|index| {
                batch
                    .messages
                    .get(*index)
                    .cloned()
                    .map(StoredLine::into_shown)
            }));
            if !absorbed_multiline.contains(&candidate) {
                let replay_kind = batch.replay.then(|| types[&candidate].clone());
                match (incomplete.get(&candidate), replay_kind) {
                    (Some(line), Some(kind)) => messages.push((line.clone(), Some(kind), None)),
                    (Some(line), None) => {
                        incomplete_events
                            .push(BatchEvent::Incomplete(line.clone(), batch.multiline_sender));
                    }
                    (None, kind) if batch.replay || batch.kind != cap::MULTILINE.as_str() => {
                        messages.extend(fallback_rows(batch.messages, &batch.routed_members, kind));
                    }
                    _ => {}
                }
            }
            for event in batch.completed {
                collect_messages(event, &mut messages);
            }
        }
        if let Some(target) = history_target {
            history_incomplete.extend(incomplete.values().cloned());
            vec![BatchEvent::History {
                target,
                items: history_items_with_incomplete(
                    messages.into_iter().map(|(line, _, _)| line).collect(),
                    history_incomplete,
                ),
                exhausted: false,
            }]
        } else {
            incomplete_events.extend(messages.into_iter().map(|(line, kind, sender)| {
                BatchEvent::Line {
                    batch_type: kind.or_else(|| {
                        line.as_ref()
                            .tag("batch")
                            .and_then(|reference| types.get(reference.as_ref()))
                            .cloned()
                    }),
                    line,
                    sender,
                }
            }));
            incomplete_events
        }
    }

    fn remember_abandoned(&mut self, reference: &str, batch: AbandonedBatch) {
        if self.abandoned.len() == MAX_ABANDONED_BATCHES {
            // An evicted batch could be either live or a replay.
            self.provenance_exhausted = true;
        } else {
            self.abandoned.insert(reference.to_owned(), batch);
        }
    }

    fn aggregate_references(&self, reference: &str) -> Vec<String> {
        let ancestors = self.ancestors(reference);
        let replay = self.open.get(reference).is_some_and(|batch| batch.replay);
        let same_provenance = |reference: &&String| self.open[*reference].replay == replay;
        let mut aggregates = Vec::new();
        let multiline = ancestors
            .iter()
            .filter(same_provenance)
            .find(|reference| self.open[*reference].kind == cap::MULTILINE.as_str());
        if let Some(reference) = multiline.or_else(|| {
            ancestors
                .iter()
                .rev()
                .filter(same_provenance)
                .find(|reference| self.open[*reference].is_typed_aggregate())
        }) {
            aggregates.push(reference.clone());
            if multiline.is_some() {
                return aggregates;
            }
        }
        for reference in ancestors {
            let batch = &self.open[&reference];
            // A labelled response may expose replay in its raw lines.
            if batch.label.is_some()
                && (batch.replay == replay || batch.kind == cap::LABELED_RESPONSE.as_str())
                && !aggregates.contains(&reference)
            {
                aggregates.push(reference);
            }
        }
        aggregates
    }

    fn replace_multiline_placeholders(
        &mut self,
        reference: &str,
        collectors: &[String],
        replacement: &[Line],
        incomplete: bool,
        sender: Option<ReplyToken>,
    ) {
        let opening = format!("+{reference}");
        for collector in collectors {
            let Some(batch) = self.open.get_mut(collector) else {
                continue;
            };
            let Some(index) = batch.messages.iter().position(|message| {
                let message = message.as_ref();
                message.command_is("BATCH") && message.param(0) == Some(opening.as_str())
            }) else {
                continue;
            };
            let previous = retained_incoming(batch.messages[index].incoming());
            let replacement_bytes = replacement
                .iter()
                .map(|message| retained_bytes(&message.as_ref()))
                .sum::<usize>()
                .saturating_add(
                    usize::from(sender.is_some()) * std::mem::size_of::<(usize, RoutedMember)>(),
                );
            let replacement_len = replacement.len();
            batch.messages.splice(
                index..=index,
                replacement.iter().cloned().map(StoredLine::from),
            );
            let route = batch.routed_members.remove(&index);
            let route = route.or_else(|| sender.map(RoutedMember::observed));
            let shifted = batch.routed_members.split_off(&index);
            batch.routed_members.extend(
                shifted
                    .into_iter()
                    .map(|(position, route)| (position - 1 + replacement_len, route)),
            );
            if let Some(route) = route {
                batch
                    .routed_members
                    .extend((index..index + replacement_len).map(|index| (index, route)));
            }
            for stored in &mut batch.incomplete {
                if *stored > index {
                    *stored = stored.saturating_sub(1).saturating_add(replacement_len);
                }
            }
            if incomplete {
                batch.incomplete.extend(index..index + replacement_len);
            }
            if replacement_bytes >= previous {
                let added = replacement_bytes - previous;
                batch.buffered_bytes = batch.buffered_bytes.saturating_add(added);
                self.buffered_bytes = self.buffered_bytes.saturating_add(added);
            } else {
                let removed = previous - replacement_bytes;
                batch.buffered_bytes = batch.buffered_bytes.saturating_sub(removed);
                self.buffered_bytes = self.buffered_bytes.saturating_sub(removed);
            }
        }
    }

    fn has_typed_aggregate_ancestor<'a>(
        &'a self,
        mut parent: Option<&'a str>,
        replay: bool,
    ) -> bool {
        while let Some(reference) = parent {
            let Some(batch) = self.open.get(reference) else {
                return false;
            };
            if batch.replay != replay {
                return false;
            }
            if batch.is_typed_aggregate() {
                return true;
            }
            parent = batch.parent.as_deref();
        }
        false
    }

    pub(crate) fn live_multiline_opener<'a>(
        &self,
        message: &LineRef<'a>,
    ) -> Option<(&'a str, &'a str)> {
        if !message.command_is("BATCH") {
            return None;
        }
        let reference = message.param(0)?.strip_prefix('+')?;
        let batch = self.open.get(reference)?;
        (!batch.replay && batch.kind == cap::MULTILINE.as_str())
            .then_some((reference, message.param(2)?))
    }

    pub(crate) fn multiline_opening(&self, message: &LineRef<'_>) -> Option<(&str, Option<&Line>)> {
        if !matches!(message.command(), Command::Privmsg | Command::Notice) {
            return None;
        }
        let reference = message.tag("batch")?;
        let (mut reference, mut batch) = self.open.get_key_value(reference.as_ref())?;
        while batch.kind != cap::MULTILINE.as_str() {
            (reference, batch) = self.open.get_key_value(batch.parent.as_ref()?)?;
        }
        let opening = batch.opening.as_ref();
        let first_valid = batch.messages.is_empty()
            && message.param(1).is_some()
            && opening
                .param(2)
                .is_none_or(|target| message.param(0) == Some(target))
            && (!matches!(opening.source(), Some(papo_proto::Source::User { .. }))
                || opening.raw_source() == message.raw_source());
        Some((reference, first_valid.then_some(&batch.opening)))
    }

    pub(crate) fn multiline_sender(&self, reference: &str) -> Option<ReplyToken> {
        self.open.get(reference)?.multiline_sender
    }

    pub(crate) fn remember_multiline_sender(&mut self, reference: &str, token: ReplyToken) {
        self.open
            .get_mut(reference)
            .expect("resolved multiline")
            .multiline_sender = Some(token);
    }

    pub(crate) fn live_member(&self, message: &LineRef<'_>) -> Option<LiveMember<'_>> {
        let reference = message.tag("batch")?;
        let (reference, batch) = self.open.get_key_value(reference.as_ref())?;
        if batch.replay {
            return None;
        }
        let split_kind = match message.command() {
            Command::Quit => Some("netsplit"),
            Command::Join => Some("netjoin"),
            _ => None,
        };
        let mut member = LiveMember {
            kind: &batch.kind,
            label: None,
            split: None,
            raw: !batch.is_typed_aggregate() && batch.kind != cap::LABELED_RESPONSE.as_str(),
        };
        let mut ancestor = Some((reference, batch));
        while let Some((reference, batch)) = ancestor {
            if member.label.is_none() {
                member.label = batch.label.as_deref();
            }
            if member.split.is_none() && split_kind == Some(batch.kind.as_str()) {
                member.split = Some(SplitServers {
                    batch: reference.clone().into(),
                    first: batch.params.first().cloned().unwrap_or_default(),
                    second: batch.params.get(1).cloned().unwrap_or_default(),
                });
            }
            ancestor = batch
                .parent
                .as_ref()
                .and_then(|parent| self.open.get_key_value(parent));
        }
        Some(member)
    }

    fn context(&self, message: &LineRef<'_>) -> DispatchContext {
        let reference = message.tag("batch");
        let live_input = reference.is_none();
        let batch_type = reference.and_then(|reference| {
            self.open
                .get(reference.as_ref())
                .map(|batch| dispatch_kind(&batch.kind, batch.replay))
                .or_else(|| {
                    self.abandoned
                        .get(reference.as_ref())
                        .map(|batch| dispatch_kind(&batch.kind, batch.replay))
                })
                .map(str::to_owned)
        });
        DispatchContext {
            batch_type,
            live_input,
            ..DispatchContext::default()
        }
    }

    pub(crate) fn recount(&self) -> usize {
        self.open
            .values()
            .map(|batch| {
                retained_bytes(&batch.opening.as_ref())
                    + batch
                        .messages
                        .iter()
                        .map(|line| retained_incoming(line.incoming()))
                        .sum::<usize>()
                    + batch
                        .completed
                        .len()
                        .saturating_mul(std::mem::size_of::<BatchEvent>())
            })
            .sum()
    }

    pub(crate) fn accounting_is_sound(&self) -> bool {
        self.buffered_bytes >= self.recount()
    }
}

fn is_history(kind: &str) -> bool {
    kind == cap::CHATHISTORY_BATCH
        || kind == cap::CHATHISTORY.as_str()
        || kind == "draft/chathistory-targets"
        || kind == cap::EVENT_PLAYBACK.as_str()
}

fn dispatch_kind(kind: &str, replay: bool) -> &str {
    if replay && !is_replay(kind) {
        cap::CHATHISTORY_BATCH
    } else {
        kind
    }
}

/// Returns whether this batch type carries replayed traffic.
#[must_use]
pub fn is_replay(kind: &str) -> bool {
    is_history(kind) || matches!(kind, "znc.in/playback" | "soju.im/search")
}

fn is_aggregate(kind: &str, bouncer_enabled: bool) -> bool {
    is_history(kind)
        || (cfg!(feature = "unstable") && bouncer_enabled && kind == cap::BOUNCER_NETWORKS_NAME)
        || kind == cap::MULTILINE.as_str()
        || matches!(
            kind,
            "netsplit" | "netjoin" | "metadata" | "metadata-subs" | "draft/isupport"
        )
}

impl OpenBatch {
    fn abandoned(&self, reason: BatchAbandonReason) -> AbandonedBatch {
        AbandonedBatch {
            kind: self.kind.clone(),
            parent: self.parent.clone(),
            reason,
            replay: self.replay,
        }
    }

    fn is_typed_aggregate(&self) -> bool {
        self.typed_aggregate
    }
}

fn complete(batch: &mut OpenBatch) -> Vec<BatchEvent> {
    #[cfg(feature = "unstable")]
    if batch.kind == cap::BOUNCER_NETWORKS.as_str() {
        return vec![BatchEvent::Bouncer(std::mem::take(&mut batch.messages))];
    }
    if batch.kind == "draft/isupport" {
        return vec![BatchEvent::Isupport(std::mem::take(&mut batch.messages))];
    }
    if matches!(batch.kind.as_str(), "metadata" | "metadata-subs") {
        return vec![BatchEvent::Metadata {
            kind: batch.kind.clone(),
            messages: std::mem::take(&mut batch.messages),
        }];
    }
    if batch.kind == cap::MULTILINE.as_str() {
        let assembled = multiline(batch);
        if batch.replay && batch.multiline_collectors.is_empty() {
            return multiline_rows(batch, assembled)
                .into_iter()
                .map(|line| BatchEvent::Line {
                    line,
                    batch_type: Some(cap::CHATHISTORY_BATCH.to_owned()),
                    sender: None,
                })
                .collect();
        }
        return assembled
            .into_iter()
            .map(|line| BatchEvent::Multiline(line, batch.multiline_sender))
            .collect();
    }
    if batch.kind == "draft/chathistory-targets" {
        return vec![BatchEvent::HistoryTargets(
            shown_rows(std::mem::take(&mut batch.messages)).collect(),
        )];
    }
    if is_history(&batch.kind) {
        let items = history_items_with_indices(
            shown_rows(std::mem::take(&mut batch.messages)).collect(),
            &std::mem::take(&mut batch.incomplete),
        );
        return vec![BatchEvent::History {
            target: batch.params.first().cloned().unwrap_or_default(),
            exhausted: batch.exhausted || items.is_empty(),
            items,
        }];
    }
    shown_rows(std::mem::take(&mut batch.messages))
        .map(|line| BatchEvent::Line {
            line,
            batch_type: Some(batch.kind.clone()),
            sender: None,
        })
        .collect()
}

pub(crate) fn parse_history_targets(rows: &[Line]) -> Vec<(String, papo_proto::ServerTime)> {
    rows.iter()
        .filter_map(|row| {
            let row = row.as_ref();
            if !row.command_is("CHATHISTORY") || row.param(0) != Some("TARGETS") {
                return None;
            }
            Some((
                row.param(1)?.to_owned(),
                papo_proto::ServerTime::from_stored(row.param(2)?).ok()?,
            ))
        })
        .collect()
}

fn multiline_rows(batch: &OpenBatch, assembled: Option<Line>) -> Vec<Line> {
    match assembled {
        Some(line) => vec![line],
        None if batch.replay => shown_rows(batch.messages.clone()).collect(),
        None => Vec::new(),
    }
}

fn multiline(batch: &OpenBatch) -> Option<Line> {
    let first = batch.messages.first()?.as_ref();
    if !matches!(first.command(), Command::Privmsg | Command::Notice) {
        return None;
    }
    let command = first.raw_command();
    let source = first.raw_source();
    let target = batch
        .params
        .first()
        .map(String::as_str)
        .or_else(|| first.param(0))?;
    if !batch.messages.iter().all(|message| {
        let message = message.as_ref();
        message.command_is(command)
            && message.param(0) == Some(target)
            && message.param(1).is_some()
            && message.raw_source() == source
    }) {
        return None;
    }
    let mut text = String::new();
    for (index, line) in batch.messages.iter().enumerate() {
        let line = line.as_ref();
        if index != 0 && !line.tags().any(|(key, _)| key == "draft/multiline-concat") {
            text.push('\n');
        }
        text.push_str(line.param(1).expect("validated multiline text"));
    }
    let opening = batch.opening.as_ref();
    // Members are the actual messages and therefore authoritative for their sender. Servers may
    // open replayed multiline batches themselves, but a user opener that names a different sender
    // is inconsistent, just like mixed member commands or targets, and is not assembled.
    if matches!(opening.source(), Some(papo_proto::Source::User { .. }))
        && opening.raw_source() != source
    {
        return None;
    }
    let mut message = Line::new(command).param(target).param(&text);
    if let Some(source) = source {
        message = message.with_source(source);
    }
    for (key, value) in opening
        .tags()
        .chain(first.tags())
        .filter(|(key, _)| !matches!(*key, "batch" | "draft/multiline-concat"))
    {
        if !message.as_ref().tags().any(|(stored, _)| stored == key) {
            message = message.with_tag(key, value.as_deref().unwrap_or_default());
        }
    }
    Some(message)
}

fn history_from_event(event: BatchEvent) -> Option<(String, Vec<HistoryItem>, bool)> {
    let BatchEvent::History {
        target,
        items,
        exhausted,
    } = event
    else {
        return None;
    };
    Some((target, items, exhausted))
}

fn unapplied_messages(
    messages: Vec<StoredLine>,
    routed: &RoutedMembers,
) -> impl Iterator<Item = StoredLine> {
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (!routed.get(&index).is_some_and(|member| member.live)).then_some(line)
        })
}

type BatchRow = (Line, Option<String>, Option<ReplyToken>);

fn fallback_rows(
    messages: Vec<StoredLine>,
    routed_members: &RoutedMembers,
    replay_kind: Option<String>,
) -> impl Iterator<Item = BatchRow> {
    messages
        .into_iter()
        .enumerate()
        .filter_map(move |(index, line)| {
            let route = routed_members.get(&index);
            let kind = match route {
                Some(member) if member.live || member.replay_copy => return None,
                Some(member) if member.sender.is_none() => Some(cap::CHATHISTORY_BATCH.to_owned()),
                _ => replay_kind.clone(),
            };
            let line = line.into_shown();
            Some((line, kind, route.and_then(|member| member.sender)))
        })
}

fn collect_messages(event: BatchEvent, messages: &mut Vec<BatchRow>) {
    match event {
        BatchEvent::Response {
            messages: batch,
            routed_members,
            replay,
            batch_type,
            history,
            history_targets,
            ..
        } => {
            if !replay || (history.is_none() && history_targets.is_none()) {
                messages.extend(fallback_rows(
                    batch,
                    &routed_members,
                    replay.then_some(batch_type),
                ));
            }
            if let Some((target, items, exhausted)) = history {
                collect_messages(
                    BatchEvent::History {
                        target,
                        items,
                        exhausted,
                    },
                    messages,
                );
            }
            if let Some(rows) = history_targets {
                collect_messages(BatchEvent::HistoryTargets(rows), messages);
            }
        }
        BatchEvent::Metadata {
            messages: batch, ..
        }
        | BatchEvent::Isupport(batch) => {
            messages.extend(shown_rows(batch).map(|line| (line, None, None)));
        }
        #[cfg(feature = "unstable")]
        BatchEvent::Bouncer(batch) => {
            messages.extend(shown_rows(batch).map(|line| (line, None, None)));
        }
        BatchEvent::History { items: batch, .. } => {
            messages.extend(batch.into_iter().map(|item| {
                (
                    item.into_line().line,
                    Some(cap::CHATHISTORY_BATCH.to_owned()),
                    None,
                )
            }));
        }
        BatchEvent::Line {
            line,
            batch_type,
            sender,
        } => messages.push((line, batch_type, sender)),
        BatchEvent::Multiline(line, sender) | BatchEvent::Incomplete(line, sender) => {
            messages.push((line, None, sender));
        }
        BatchEvent::HistoryTargets(rows) => messages.extend(
            rows.into_iter()
                .map(|line| (line, Some("draft/chathistory-targets".to_owned()), None)),
        ),
        BatchEvent::Abandoned { .. } => {}
    }
}

fn history_items_with_incomplete(
    messages: Vec<Line>,
    mut incomplete: Vec<Line>,
) -> Vec<HistoryItem> {
    history_items(messages)
        .into_iter()
        .map(|item| {
            let HistoryItem::Message(line) = item else {
                return item;
            };
            if let Some(index) = incomplete.iter().position(|partial| partial == &line.line) {
                incomplete.remove(index);
                HistoryItem::Incomplete(line)
            } else {
                HistoryItem::Message(line)
            }
        })
        .collect()
}

fn history_items_with_indices(messages: Vec<Line>, incomplete: &[usize]) -> Vec<HistoryItem> {
    let mut msgids = HashSet::new();
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let parsed = message.as_ref();
            let msgid = parsed.tag("msgid").map(std::borrow::Cow::into_owned);
            let context = parsed
                .tags()
                .any(|(key, _)| key == "draft/chathistory-context");
            msgid.is_none_or(|msgid| msgids.insert(msgid)).then(|| {
                let item = HistoryLine {
                    line: message,
                    context,
                    actor_party: crate::Party::Unknown,
                    subject_party: crate::Party::Unknown,
                };
                if incomplete.contains(&index) {
                    HistoryItem::Incomplete(item)
                } else {
                    classify_history_item(item)
                }
            })
        })
        .collect()
}

fn history_items(messages: Vec<Line>) -> Vec<HistoryItem> {
    let mut msgids = HashSet::new();
    messages
        .into_iter()
        .filter_map(|message| {
            let parsed = message.as_ref();
            let msgid = parsed.tag("msgid").map(std::borrow::Cow::into_owned);
            let context = parsed
                .tags()
                .any(|(key, _)| key == "draft/chathistory-context");
            msgid.is_none_or(|msgid| msgids.insert(msgid)).then(|| {
                classify_history_item(HistoryLine {
                    line: message,
                    context,
                    actor_party: crate::Party::Unknown,
                    subject_party: crate::Party::Unknown,
                })
            })
        })
        .collect()
}

fn classify_history_item(item: HistoryLine) -> HistoryItem {
    match item.line.as_ref().command() {
        Command::Privmsg | Command::Notice => return HistoryItem::Message(item),
        Command::Join => return HistoryItem::Joined(item),
        Command::Part => return HistoryItem::Parted(item),
        Command::Quit => return HistoryItem::Quit(item),
        Command::Kick => return HistoryItem::Kicked(item),
        Command::Nick => return HistoryItem::NickChanged(item),
        Command::Topic => return HistoryItem::TopicChanged(item),
        Command::Mode => return HistoryItem::ModeChanged(item),
        _ => {}
    }
    if item.line.as_ref().command_is("TAGMSG") {
        return if Reaction::parse(&item.line.as_ref()).is_some() {
            HistoryItem::Reacted(item)
        } else {
            HistoryItem::TagMsg(item)
        };
    }
    if item.line.as_ref().command_is("REDACT") {
        return HistoryItem::Redacted(item);
    }
    HistoryItem::Other(item)
}

fn valid_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn shown_rows(messages: Vec<StoredLine>) -> impl Iterator<Item = Line> {
    messages.into_iter().map(StoredLine::into_shown)
}

fn retained_incoming(incoming: IncomingLine<'_>) -> usize {
    retained_bytes(&incoming.shown) + incoming.original_bytes(retained_bytes)
}

fn retained_bytes(message: &LineRef<'_>) -> usize {
    let tags = message
        .tags()
        .map(|(key, value)| {
            std::mem::size_of::<(String, Option<String>)>()
                + key.len()
                + value.as_deref().map_or(0, str::len)
        })
        .sum::<usize>();
    std::mem::size_of::<Line>()
        + tags
        + message.raw_source().map_or(0, str::len)
        + message.raw_command().len()
        + message
            .params()
            .map(|param| std::mem::size_of::<String>() + param.len())
            .sum::<usize>()
}

#[cfg(test)]
#[path = "batch/state_machine_tests.rs"]
mod state_machine_tests;

#[cfg(test)]
mod tests {
    use super::{BatchAction, BatchEvent, Batches, retained_bytes};
    use crate::{BatchAbandonReason, Config};
    use papo_proto::{Line, LineRef};
    use web_time::Instant;

    fn handle(batches: &mut Batches, line: &str) -> (bool, Option<String>) {
        let message = LineRef::parse(line).unwrap();
        match batches.handle(Instant::now(), &message, usize::MAX, false, false, None) {
            BatchAction::Complete(mut events) => match events.pop() {
                Some(BatchEvent::Line { batch_type, .. }) => (false, batch_type),
                _ => (false, None),
            },
            BatchAction::Consumed => (false, None),
            BatchAction::Dispatch(context) => (true, context.batch_type().map(str::to_owned)),
        }
    }

    #[test]
    fn applied_state_members_remain_raw_response_lines() {
        let now = Instant::now();
        let mut batches = Batches::new(&Config::default().max_batch_lines(1));
        let mut handle = |line, state| {
            batches.handle(
                now,
                &LineRef::parse(line).unwrap(),
                usize::MAX,
                false,
                true,
                state,
            )
        };
        _ = handle("@label=0 :ergo.test BATCH +reply labeled-response", None);
        let applied = super::RoutedMember {
            live: true,
            replay_copy: false,
            subject: None,
            joined_ready: None,
            sender: None,
        };
        assert!(matches!(
            handle(
                "@batch=reply :alice!~alice@staff.example NICK alice-work",
                Some(applied)
            ),
            BatchAction::Consumed
        ));
        let BatchAction::Complete(events) = handle(":ergo.test BATCH -reply", None) else {
            panic!("labelled batch failed to close")
        };
        assert!(
            matches!(events.as_slice(), [BatchEvent::Response { messages, routed_members, .. }]
            if messages[0].as_ref().command_is("NICK") && routed_members[&0].live)
        );
    }

    #[test]
    fn resolves_nested_and_interleaved_batch_types() {
        let mut batches = Batches::new(&Config::default());
        assert_eq!(
            handle(&mut batches, "BATCH +outer vendor/sync"),
            (false, None)
        );
        assert_eq!(
            handle(&mut batches, "BATCH +other vendor/other"),
            (false, None)
        );
        assert_eq!(
            handle(&mut batches, "@batch=outer BATCH +i nested"),
            (false, None)
        );
        assert_eq!(
            handle(&mut batches, "@batch=i :irc.example NOTICE papo :nested"),
            (false, Some("nested".into()))
        );
        assert_eq!(
            handle(&mut batches, "@batch=other :irc.example NOTICE papo :other"),
            (false, Some("vendor/other".into()))
        );
    }

    #[test]
    fn passes_through_invalid_envelopes() {
        let mut batches = Batches::new(&Config::default());
        assert_eq!(handle(&mut batches, "BATCH +bad_ref sync"), (true, None));
        assert_eq!(handle(&mut batches, "BATCH -missing"), (true, None));
    }

    #[test]
    fn completes_history_with_its_members() {
        let mut batches = Batches::new(&Config::default());
        assert_eq!(
            handle(&mut batches, "BATCH +h chathistory #papo"),
            (false, None)
        );
        assert_eq!(
            handle(&mut batches, "@batch=h :alice PRIVMSG #papo :hello"),
            (false, None)
        );
        let close = LineRef::parse("BATCH -h").unwrap();
        let BatchAction::Complete(events) =
            batches.handle(Instant::now(), &close, usize::MAX, false, false, None)
        else {
            panic!("history did not complete")
        };
        let BatchEvent::History { target, items, .. } = &events[0] else {
            panic!("expected history")
        };
        assert_eq!(target, "#papo");
        assert_eq!(
            items[0].as_message().unwrap().as_ref().param(1),
            Some("hello")
        );
    }

    #[test]
    fn bounds_refused_batch_tombstones_and_clears_them_on_reset() {
        let mut batches = Batches::new(&Config::default().max_open_batches(0));
        for index in 0..10_000 {
            let line = format!("BATCH +ref{index} vendor/sync");
            let message = LineRef::parse(&line).unwrap();
            assert!(matches!(
                batches.handle(Instant::now(), &message, usize::MAX, false, false, None),
                BatchAction::Complete(_)
            ));
            assert_eq!(batches.provenance_exhausted, index >= 256);
        }
        assert_eq!(batches.abandoned.len(), 256);

        assert!(batches.provenance_exhausted);
        let late = LineRef::parse("@batch=ref0 :alice PART #papo :Leaving").unwrap();
        let BatchAction::Dispatch(context) =
            batches.handle(Instant::now(), &late, usize::MAX, false, false, None)
        else {
            panic!("expected a late live member");
        };
        assert!(context.live_input);
        assert_eq!(context.batch_type(), None);

        batches.reset();
        assert!(batches.abandoned.is_empty());
        assert!(!batches.provenance_exhausted);
    }

    #[test]
    fn correlated_label_batch_hits_the_global_byte_budget() {
        let now = Instant::now();
        let byte_budget = 4 * 1024;
        let config = Config::default().__batch_byte_budget(byte_budget);
        let mut batches = Batches::new(&config);
        let open = LineRef::parse("@label=lost BATCH +reply labeled-response").unwrap();
        assert!(matches!(
            batches.handle(now, &open, usize::MAX, false, true, None),
            BatchAction::Consumed
        ));
        let prefix = "@batch=reply;vendor.example/large=";
        let suffix = " :alice!~alice@users.example PRIVMSG #papo :payload";
        let wire = format!("{prefix}{}{suffix}", "x".repeat(511 - prefix.len() - 1));
        let member = LineRef::parse(&wire).unwrap();
        let mut fed = 0;

        loop {
            fed += wire.len();
            if let BatchAction::Complete(events) =
                batches.handle(now, &member, usize::MAX, false, true, None)
            {
                assert!(events.iter().any(|event| matches!(
                    event,
                    BatchEvent::Abandoned {
                        reason: BatchAbandonReason::ByteLimit,
                        ..
                    }
                )));
                break;
            }
        }

        assert!(fed < byte_budget);
    }

    #[test]
    fn retained_replies_count_both_the_reported_and_routing_parameters() {
        let shown = LineRef::parse(":ergo.test 696 papo #staff k <redacted>").unwrap();
        let original = LineRef::parse(":ergo.test 696 papo #staff k staff-key-4821").unwrap();
        let incoming = super::IncomingLine::new(shown, Some(original));
        assert_eq!(
            super::retained_incoming(incoming),
            retained_bytes(&shown) + retained_bytes(&original)
        );
    }

    #[test]
    fn retained_bytes_counts_every_owned_line_component() {
        let message = LineRef::parse(
            "@time=2026-10-15T12:00:01.000Z;msgid=RKi9Cw;+draft/read-marker \
             :alice!~alice@users.example PRIVMSG #papo :review is ready",
        )
        .unwrap();
        let expected = std::mem::size_of::<Line>()
            + 3 * std::mem::size_of::<(String, Option<String>)>()
            + "time".len()
            + "2026-10-15T12:00:01.000Z".len()
            + "msgid".len()
            + "RKi9Cw".len()
            + "+draft/read-marker".len()
            + "alice!~alice@users.example".len()
            + "PRIVMSG".len()
            + 2 * std::mem::size_of::<String>()
            + "#papo".len()
            + "review is ready".len();

        assert_eq!(retained_bytes(&message), expected);
    }

    #[test]
    fn recount_matches_nested_history_storage_and_detects_a_shortfall() {
        let mut batches = Batches::new(&Config::default());
        let history = LineRef::parse(":ergo.test BATCH +history chathistory #papo").unwrap();
        let multiline = LineRef::parse(
            "@batch=history;msgid=RKi9Cw;time=2026-10-15T12:00:01.000Z \
             :alice!~alice@users.example BATCH +multiline draft/multiline #papo",
        )
        .unwrap();
        let message = LineRef::parse(
            "@batch=multiline :alice!~alice@users.example PRIVMSG #papo :review is ready",
        )
        .unwrap();

        for line in [&history, &multiline, &message] {
            assert!(matches!(
                batches.handle(Instant::now(), line, usize::MAX, false, false, None),
                BatchAction::Consumed
            ));
        }

        let expected =
            retained_bytes(&history) + 2 * retained_bytes(&multiline) + retained_bytes(&message);
        assert_eq!(batches.recount(), expected);
        assert_eq!(batches.buffered_bytes, expected + retained_bytes(&message));
        assert!(batches.accounting_is_sound());
        assert_eq!(handle(&mut batches, "BATCH -multiline"), (false, None));
        assert!(batches.accounting_is_sound());
        batches.buffered_bytes = batches.recount() - 1;
        assert!(!batches.accounting_is_sound());
    }

    #[test]
    fn placeholder_removal_shifts_only_later_incomplete_indices() {
        let mut batches = Batches::new(&Config::default());
        for line in [
            ":ergo.test BATCH +history chathistory #papo",
            "@batch=history :alice!~alice@users.example PRIVMSG #papo :release queued",
            "@batch=history :alice!~alice@users.example PRIVMSG #papo :release started",
            "@batch=history :ergo.test BATCH +deploy draft/multiline #papo",
            "@batch=history :alice!~alice@users.example PRIVMSG #papo :release complete",
        ] {
            assert_eq!(handle(&mut batches, line), (false, None));
        }
        let history = batches.open.get_mut("history").unwrap();
        assert_eq!(history.messages.len(), 4);
        history.incomplete = vec![1, 2, 3];

        batches.replace_multiline_placeholders("deploy", &["history".to_owned()], &[], false, None);

        let history = &batches.open["history"];
        assert_eq!(history.messages.len(), 3);
        assert_eq!(history.incomplete, [1, 2, 2]);
    }

    #[test]
    fn completed_child_byte_budget_accepts_the_boundary_and_rejects_one_over() {
        let byte_budget = 4 * 1024;
        for (total, abandoned) in [(byte_budget, false), (byte_budget + 1, true)] {
            let config = Config::default()
                .max_batch_lines(1)
                .__batch_byte_budget(byte_budget);
            let mut batches = Batches::new(&config);
            assert_eq!(
                handle(&mut batches, "BATCH +outer vendor/sync"),
                (false, None)
            );
            assert_eq!(
                handle(&mut batches, "@batch=outer BATCH +inner metadata"),
                (false, None)
            );
            let completed_bytes = std::mem::size_of::<BatchEvent>();
            let padding = total - batches.buffered_bytes - completed_bytes;
            batches.buffered_bytes += padding;
            batches.open.get_mut("outer").unwrap().buffered_bytes += padding;

            let close = LineRef::parse("@batch=outer BATCH -inner").unwrap();
            let action = batches.handle(Instant::now(), &close, usize::MAX, false, false, None);
            assert_eq!(
                matches!(action, BatchAction::Complete(ref events) if events.iter().any(|event| matches!(
                    event,
                    BatchEvent::Abandoned { reason: BatchAbandonReason::ByteLimit, .. }
                ))),
                abandoned,
                "retained total {total}"
            );
            if !abandoned {
                assert!(matches!(action, BatchAction::Consumed));
                assert_eq!(batches.buffered_bytes, byte_budget);
            }
        }
    }

    #[test]
    fn multiline_replacement_charges_the_sender_at_the_byte_limit() {
        let byte_budget = 67_108_864;
        let sender = crate::ReplyToken::__unknown(papo_proto::Origin::NONE);
        let sender_entry = std::mem::size_of::<(usize, super::RoutedMember)>();
        let opener = "@batch=reply :alice!~alice@users.example BATCH +deploy draft/multiline #papo";
        let assembled = ":alice!~alice@users.example PRIVMSG #papo :release is ready";
        let placeholder_bytes = retained_bytes(&LineRef::parse(opener).unwrap());
        let assembled_bytes = retained_bytes(&LineRef::parse(assembled).unwrap());

        for (sender, sender_bytes) in [(None, 0), (Some(sender), sender_entry)] {
            for (total, abandoned) in [(byte_budget, false), (byte_budget + 1, true)] {
                let mut batches = Batches::new(&Config::default());
                let now = Instant::now();
                let handle = |batches: &mut Batches, line| {
                    batches.handle(
                        now,
                        &LineRef::parse(line).unwrap(),
                        usize::MAX,
                        false,
                        true,
                        None,
                    )
                };
                for line in [
                    "@label=deploy :ergo.test BATCH +reply labeled-response",
                    opener,
                    "@batch=deploy :alice!~alice@users.example PRIVMSG #papo :release is ready",
                ] {
                    assert!(matches!(handle(&mut batches, line), BatchAction::Consumed));
                }
                if let Some(sender) = sender {
                    batches.remember_multiline_sender("deploy", sender);
                }

                // Model other retained rows without allocating 64 MiB in each case.
                let after_replacement =
                    batches.buffered_bytes - placeholder_bytes + assembled_bytes + sender_bytes;
                let padding = total - after_replacement;
                batches.buffered_bytes += padding;
                batches.open.get_mut("reply").unwrap().buffered_bytes += padding;

                let action = handle(&mut batches, "@batch=reply :ergo.test BATCH -deploy");
                if abandoned {
                    assert!(matches!(
                        action,
                        BatchAction::Complete(events) if events.iter().any(|event| matches!(
                            event, BatchEvent::Abandoned { reason: BatchAbandonReason::ByteLimit, .. }
                        ))
                    ));
                } else {
                    assert!(matches!(action, BatchAction::Consumed));
                    assert_eq!(batches.buffered_bytes, byte_budget);
                }
            }
        }
    }

    #[test]
    fn default_batch_byte_budget_is_sixty_four_mibibytes() {
        let batches = Batches::new(&Config::default());

        assert_eq!(batches.byte_budget, 67_108_864);
    }

    #[test]
    fn global_batch_byte_limit_accepts_the_boundary_and_rejects_one_over() {
        let line = "@batch=sync;time=2026-10-15T12:00:01.000Z \
                    :alice!~alice@users.example PRIVMSG #papo :review is ready";
        let retained = retained_bytes(&LineRef::parse(line).unwrap());

        let byte_budget = 4 * 1024;
        for (total, abandoned) in [(byte_budget, false), (byte_budget + 1, true)] {
            let config = Config::default().__batch_byte_budget(byte_budget);
            let mut batches = Batches::new(&config);
            assert_eq!(
                handle(&mut batches, "BATCH +sync vendor/sync"),
                (false, None)
            );
            batches.buffered_bytes = total - retained;

            let action = batches.handle(
                Instant::now(),
                &LineRef::parse(line).unwrap(),
                usize::MAX,
                false,
                false,
                None,
            );
            assert_eq!(
                matches!(action, BatchAction::Complete(ref events) if events.iter().any(|event| matches!(
                    event,
                    BatchEvent::Abandoned { reason: BatchAbandonReason::ByteLimit, .. }
                ))),
                abandoned,
                "retained total {total}"
            );
        }
    }

    #[test]
    fn labeled_response_aggregates_only_for_a_pending_label() {
        for (pending, expect_response) in [(false, false), (true, true)] {
            let now = Instant::now();
            let mut batches = Batches::new(&Config::default());
            let open = LineRef::parse("@label=request-7 BATCH +reply labeled-response").unwrap();
            assert!(matches!(
                batches.handle(now, &open, usize::MAX, false, pending, None),
                BatchAction::Consumed
            ));
            let member =
                LineRef::parse("@batch=reply :irc.example 401 papo missing :No such nick").unwrap();
            let _ = batches.handle(now, &member, usize::MAX, false, pending, None);
            let close = LineRef::parse("BATCH -reply").unwrap();
            let BatchAction::Complete(events) =
                batches.handle(now, &close, usize::MAX, false, pending, None)
            else {
                panic!("labeled response did not complete")
            };

            assert_eq!(
                events
                    .iter()
                    .any(|event| matches!(event, BatchEvent::Response { .. })),
                expect_response
            );
        }
    }
}
