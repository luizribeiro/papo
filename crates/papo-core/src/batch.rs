use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    time::Duration,
};
use web_time::Instant;

use papo_proto::{Command, Line, LineRef, Reaction, time};

use crate::{BatchAbandonReason, Config, HistoryItem, HistoryLine, cap};

const MAX_ABANDONED_BATCHES: usize = 256;
const MAX_BUFFERED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct Batches {
    open: BTreeMap<String, OpenBatch>,
    abandoned: VecDeque<(String, String)>,
    max_open: usize,
    max_lines: usize,
    timeout: Duration,
    buffered_bytes: usize,
}

#[derive(Debug)]
struct OpenBatch {
    opening: Line,
    kind: String,
    label: Option<String>,
    parent: Option<String>,
    params: Vec<String>,
    end: bool,
    messages: Vec<Line>,
    completed: Vec<BatchEvent>,
    lines: usize,
    multiline_bytes: usize,
    buffered_bytes: usize,
    opened: Instant,
    typed_aggregate: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DispatchContext {
    batch_type: Option<String>,
}

impl DispatchContext {
    pub(crate) fn batch_type(&self) -> Option<&str> {
        self.batch_type.as_deref()
    }

    pub(crate) fn from_batch_type(batch_type: String) -> Self {
        Self {
            batch_type: Some(batch_type),
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
        messages: Vec<Line>,
        batch_type: String,
        history: Option<(String, Vec<HistoryItem>, bool)>,
        history_targets: Option<Vec<(String, std::time::SystemTime)>>,
    },
    History {
        target: String,
        items: Vec<HistoryItem>,
        end: bool,
    },
    HistoryTargets(Vec<(String, std::time::SystemTime)>),
    Netsplit {
        reference: String,
        servers: [String; 2],
        quits: Vec<Line>,
    },
    Netjoin {
        reference: String,
        servers: [String; 2],
        joins: Vec<Line>,
    },
    Abandoned {
        reference: String,
        batch_type: String,
        reason: BatchAbandonReason,
    },
    Line {
        line: Line,
        batch_type: Option<String>,
    },
    Multiline(Line),
    Metadata {
        kind: String,
        messages: Vec<Line>,
    },
    #[cfg(feature = "unstable")]
    Bouncer(Vec<Line>),
    Isupport(Vec<Line>),
}

impl Batches {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            open: BTreeMap::new(),
            abandoned: VecDeque::new(),
            max_open: config.max_open_batches,
            max_lines: config.max_batch_lines,
            timeout: config.batch_timeout,
            buffered_bytes: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.open.clear();
        self.abandoned.clear();
        self.buffered_bytes = 0;
    }

    pub(crate) fn handle(
        &mut self,
        now: Instant,
        message: &LineRef<'_>,
        multiline_max_bytes: usize,
        bouncer_enabled: bool,
        correlated_label: bool,
    ) -> BatchAction {
        if !message.command_is("BATCH") {
            return self.member(message, multiline_max_bytes);
        }
        let Some(reference) = message.param(0) else {
            return BatchAction::Dispatch(self.context(message));
        };
        if let Some(reference) = reference.strip_prefix('+') {
            return self.open(now, message, reference, bouncer_enabled, correlated_label);
        }
        if let Some(reference) = reference.strip_prefix('-') {
            return self.close(message, reference);
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
        if !valid_reference(reference)
            || self.open.contains_key(reference)
            || parent
                .as_ref()
                .is_some_and(|parent| !self.open.contains_key(parent))
        {
            return BatchAction::Dispatch(self.context(message));
        }
        if self.open.len() >= self.max_open {
            self.remember_abandoned(reference, kind);
            return BatchAction::Complete(vec![BatchEvent::Abandoned {
                reference: reference.to_owned(),
                batch_type: kind.to_owned(),
                reason: BatchAbandonReason::OpenBatchLimit,
            }]);
        }
        self.open.insert(
            reference.to_owned(),
            OpenBatch {
                opening: message.to_owned(),
                kind: kind.to_owned(),
                label: correlated_label
                    .then(|| message.tag("label"))
                    .flatten()
                    .map(std::borrow::Cow::into_owned),
                parent,
                params: message.params().skip(2).map(str::to_owned).collect(),
                end: message
                    .tags()
                    .any(|(key, _)| key == "draft/chathistory-end"),
                messages: Vec::new(),
                completed: Vec::new(),
                lines: 0,
                multiline_bytes: 0,
                buffered_bytes: retained_bytes(message),
                opened: now,
                typed_aggregate: is_aggregate(kind, bouncer_enabled),
            },
        );
        self.buffered_bytes = self.buffered_bytes.saturating_add(retained_bytes(message));
        BatchAction::Consumed
    }

    fn close(&mut self, message: &LineRef<'_>, reference: &str) -> BatchAction {
        let actual_parent = message.tag("batch");
        let valid = valid_reference(reference)
            && message.len() == 1
            && self.open.get(reference).is_some_and(|batch| {
                batch.parent.as_deref() == actual_parent.as_deref()
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
        if batch.is_typed_aggregate() && !self.has_typed_aggregate_ancestor(batch.parent.as_deref())
        {
            completed.extend(complete(&mut batch, reference));
        }
        if let Some(label) = batch.label.take() {
            let echo = completed.iter().find_map(|event| match event {
                BatchEvent::Multiline(message) => Some(message.clone()),
                _ => None,
            });
            let history = completed.iter().find_map(history_from_event);
            let history_targets = completed.iter().find_map(|event| match event {
                BatchEvent::HistoryTargets(targets) => Some(targets.clone()),
                _ => None,
            });
            completed.push(BatchEvent::Response {
                label,
                echo,
                messages: response_messages.expect("label captured response messages"),
                batch_type: batch.kind.clone(),
                history,
                history_targets,
            });
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
            if parent_batch.lines > self.max_lines || self.buffered_bytes > MAX_BUFFERED_BYTES {
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

    fn member(&mut self, message: &LineRef<'_>, multiline_max_bytes: usize) -> BatchAction {
        let context = self.context(message);
        let Some(reference) = message.tag("batch") else {
            return BatchAction::Dispatch(context);
        };
        if !self.open.contains_key(reference.as_ref()) {
            let batch_type =
                self.abandoned.iter().rev().find_map(|(stored, kind)| {
                    (stored == reference.as_ref()).then(|| kind.clone())
                });
            return BatchAction::Complete(vec![BatchEvent::Line {
                line: message.to_owned(),
                batch_type,
            }]);
        }
        let ancestors = self.ancestors(reference.as_ref());
        let retained = retained_bytes(message);
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
        let aggregates = message
            .tag("batch")
            .map(|reference| self.aggregate_references(reference.as_ref()))
            .unwrap_or_default();
        for reference in &aggregates {
            self.open
                .get_mut(reference)
                .expect("resolved aggregate remains open")
                .messages
                .push(message.to_owned());
        }
        if let Some(reference) = ancestors
            .iter()
            .rev()
            .find(|reference| {
                self.open[*reference].lines > self.max_lines
                    || (self.open[*reference].kind == cap::MULTILINE.as_str()
                        && self.open[*reference].multiline_bytes > multiline_max_bytes)
                    || self.buffered_bytes > MAX_BUFFERED_BYTES
            })
            .cloned()
        {
            let reason = if self.open[&reference].lines > self.max_lines {
                BatchAbandonReason::LineLimit
            } else {
                BatchAbandonReason::ByteLimit
            };
            let mut events = self.abandon(&reference, reason);
            if aggregates.is_empty() {
                let fallback = BatchEvent::Line {
                    line: message.to_owned(),
                    batch_type: context.batch_type,
                };
                events.insert(events.len() - 1, fallback);
            }
            return BatchAction::Complete(events);
        }
        if aggregates.is_empty() && context.batch_type.is_some() {
            BatchAction::Complete(vec![BatchEvent::Line {
                line: message.to_owned(),
                batch_type: context.batch_type,
            }])
        } else if aggregates.is_empty() {
            BatchAction::Dispatch(context)
        } else {
            BatchAction::Consumed
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

    fn abandon(&mut self, reference: &str, reason: BatchAbandonReason) -> Vec<BatchEvent> {
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
        let types: BTreeMap<_, _> = references
            .iter()
            .map(|candidate| (candidate.clone(), self.open[candidate].kind.clone()))
            .collect();
        let batch_type = types[reference].clone();
        let history_target = is_history(&batch_type)
            .then(|| self.open[reference].params.first().cloned())
            .flatten();
        for (candidate, kind) in &types {
            self.remember_abandoned(candidate, kind);
        }
        let mut messages = Vec::new();
        for candidate in references {
            let batch = self.open.remove(&candidate).expect("selected open batch");
            self.buffered_bytes = self.buffered_bytes.saturating_sub(batch.buffered_bytes);
            messages.extend(batch.messages);
            for event in batch.completed {
                collect_messages(event, &mut messages);
            }
        }
        let mut events = if let Some(target) = history_target {
            vec![BatchEvent::History {
                target,
                items: history_items(messages),
                end: false,
            }]
        } else {
            messages
                .into_iter()
                .map(|line| BatchEvent::Line {
                    batch_type: line
                        .as_ref()
                        .tag("batch")
                        .and_then(|reference| types.get(reference.as_ref()))
                        .cloned(),
                    line,
                })
                .collect()
        };
        events.push(BatchEvent::Abandoned {
            reference: reference.to_owned(),
            batch_type,
            reason,
        });
        events
    }

    fn remember_abandoned(&mut self, reference: &str, kind: &str) {
        self.abandoned
            .retain(|(stored, _)| stored.as_str() != reference);
        if self.abandoned.len() == MAX_ABANDONED_BATCHES {
            self.abandoned.pop_front();
        }
        self.abandoned
            .push_back((reference.to_owned(), kind.to_owned()));
    }

    fn aggregate_references(&self, reference: &str) -> Vec<String> {
        let ancestors = self.ancestors(reference);
        let mut aggregates = Vec::new();
        if let Some(reference) = ancestors
            .iter()
            .rev()
            .find(|reference| self.open[*reference].is_typed_aggregate())
        {
            aggregates.push(reference.clone());
        }
        for reference in ancestors {
            if self.open[&reference].label.is_some() && !aggregates.contains(&reference) {
                aggregates.push(reference);
            }
        }
        aggregates
    }

    fn has_typed_aggregate_ancestor<'a>(&'a self, mut parent: Option<&'a str>) -> bool {
        while let Some(reference) = parent {
            let Some(batch) = self.open.get(reference) else {
                return false;
            };
            if batch.is_typed_aggregate() {
                return true;
            }
            parent = batch.parent.as_deref();
        }
        false
    }

    fn context(&self, message: &LineRef<'_>) -> DispatchContext {
        let batch_type = message
            .tag("batch")
            .and_then(|reference| self.open.get(reference.as_ref()))
            .map(|batch| batch.kind.clone());
        DispatchContext { batch_type }
    }

    pub(crate) fn recount(&self) -> usize {
        self.open
            .values()
            .map(|batch| {
                retained_bytes(&batch.opening.as_ref())
                    + batch
                        .messages
                        .iter()
                        .map(|line| retained_bytes(&line.as_ref()))
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
    fn is_typed_aggregate(&self) -> bool {
        self.typed_aggregate
    }
}

fn complete(batch: &mut OpenBatch, reference: &str) -> Vec<BatchEvent> {
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
        return multiline(batch)
            .into_iter()
            .map(BatchEvent::Multiline)
            .collect();
    }
    if batch.kind == "draft/chathistory-targets" {
        let targets = std::mem::take(&mut batch.messages)
            .into_iter()
            .filter_map(|message| {
                let message = message.as_ref();
                if !message.command_is("CHATHISTORY") || message.param(0) != Some("TARGETS") {
                    return None;
                }
                Some((
                    message.param(1)?.to_owned(),
                    time::parse(message.param(2)?)?,
                ))
            })
            .collect();
        return vec![BatchEvent::HistoryTargets(targets)];
    }
    let servers = || {
        [
            batch.params.first().cloned().unwrap_or_default(),
            batch.params.get(1).cloned().unwrap_or_default(),
        ]
    };
    let expected_command = match batch.kind.as_str() {
        "netsplit" => "QUIT",
        "netjoin" => "JOIN",
        _ => {
            let items = history_items(std::mem::take(&mut batch.messages));
            return vec![BatchEvent::History {
                target: batch.params.first().cloned().unwrap_or_default(),
                end: batch.end || items.is_empty(),
                items,
            }];
        }
    };
    let (members, others): (Vec<_>, Vec<_>) = std::mem::take(&mut batch.messages)
        .into_iter()
        .partition(|message| message.as_ref().command_is(expected_command));
    let aggregate = if expected_command == "QUIT" {
        BatchEvent::Netsplit {
            reference: reference.to_owned(),
            servers: servers(),
            quits: members,
        }
    } else {
        BatchEvent::Netjoin {
            reference: reference.to_owned(),
            servers: servers(),
            joins: members,
        }
    };
    std::iter::once(aggregate)
        .chain(others.into_iter().map(|line| BatchEvent::Line {
            line,
            batch_type: Some(batch.kind.clone()),
        }))
        .collect()
}

fn multiline(batch: &mut OpenBatch) -> Option<Line> {
    let first = batch.messages.first()?.as_ref();
    let command = first.raw_command();
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
    let source = opening.raw_source().or_else(|| first.raw_source());
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

fn history_from_event(event: &BatchEvent) -> Option<(String, Vec<HistoryItem>, bool)> {
    let BatchEvent::History { target, items, end } = event else {
        return None;
    };
    Some((target.clone(), items.clone(), *end))
}

fn collect_messages(event: BatchEvent, messages: &mut Vec<Line>) {
    match event {
        BatchEvent::Response {
            messages: batch, ..
        }
        | BatchEvent::Metadata {
            messages: batch, ..
        }
        | BatchEvent::Isupport(batch) => messages.extend(batch),
        #[cfg(feature = "unstable")]
        BatchEvent::Bouncer(batch) => messages.extend(batch),
        BatchEvent::History { items: batch, .. } => {
            messages.extend(
                batch
                    .into_iter()
                    .map(HistoryItem::into_line)
                    .map(|item| item.line),
            );
        }
        BatchEvent::Netsplit { quits, .. } => messages.extend(quits),
        BatchEvent::Netjoin { joins, .. } => messages.extend(joins),
        BatchEvent::Line { line, .. } | BatchEvent::Multiline(line) => {
            messages.push(line);
        }
        BatchEvent::HistoryTargets(_) | BatchEvent::Abandoned { .. } => {}
    }
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
                    authorship: crate::Authorship::Unknown,
                    subject_authorship: crate::Authorship::Unknown,
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

#[cfg(all(test, not(all(target_os = "wasi", not(target_env = "p1")))))]
#[path = "batch/state_machine.rs"]
mod state_machine_tests;

#[cfg(test)]
mod tests {
    use super::{BatchAction, BatchEvent, Batches, retained_bytes};
    use crate::{BatchAbandonReason, Config};
    use papo_proto::{Line, LineRef};
    use web_time::Instant;

    fn handle(batches: &mut Batches, line: &str) -> (bool, Option<String>) {
        let message = LineRef::parse(line).unwrap();
        match batches.handle(Instant::now(), &message, usize::MAX, false, false) {
            BatchAction::Complete(mut events) => match events.pop() {
                Some(BatchEvent::Line { batch_type, .. }) => (false, batch_type),
                _ => (false, None),
            },
            BatchAction::Consumed => (false, None),
            BatchAction::Dispatch(context) => (true, context.batch_type().map(str::to_owned)),
        }
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
            batches.handle(Instant::now(), &close, usize::MAX, false, false)
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
                batches.handle(Instant::now(), &message, usize::MAX, false, false),
                BatchAction::Complete(_)
            ));
        }
        assert_eq!(batches.abandoned.len(), 256);

        let late = LineRef::parse("@batch=ref9999 :alice PART #papo :old").unwrap();
        let BatchAction::Complete(events) =
            batches.handle(Instant::now(), &late, usize::MAX, false, false)
        else {
            panic!("late refused member was dispatched live")
        };
        assert!(matches!(
            events.as_slice(),
            [BatchEvent::Line { batch_type: Some(kind), .. }] if kind == "vendor/sync"
        ));

        batches.reset();
        assert!(batches.abandoned.is_empty());
    }

    #[test]
    fn correlated_label_batch_hits_the_global_byte_budget() {
        let now = Instant::now();
        let mut batches = Batches::new(&Config::default());
        let open = LineRef::parse("@label=lost BATCH +reply labeled-response").unwrap();
        assert!(matches!(
            batches.handle(now, &open, usize::MAX, false, true),
            BatchAction::Consumed
        ));
        let prefix = "@batch=reply;vendor.example/large=";
        let suffix = " :alice!~alice@users.example PRIVMSG #papo :payload";
        let wire = format!("{prefix}{}{suffix}", "x".repeat(8_191 - prefix.len() - 1));
        let member = LineRef::parse(&wire).unwrap();
        let mut fed = 0;

        loop {
            fed += wire.len();
            if let BatchAction::Complete(events) =
                batches.handle(now, &member, usize::MAX, false, true)
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

        assert!(fed < 64 * 1024 * 1024);
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
    fn global_batch_byte_limit_accepts_the_boundary_and_rejects_one_over() {
        let line = "@batch=sync;time=2026-10-15T12:00:01.000Z \
                    :alice!~alice@users.example PRIVMSG #papo :review is ready";
        let retained = retained_bytes(&LineRef::parse(line).unwrap());

        for (total, abandoned) in [(67_108_863, false), (67_108_864, false), (67_108_865, true)] {
            let mut batches = Batches::new(&Config::default());
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
                batches.handle(now, &open, usize::MAX, false, pending),
                BatchAction::Consumed
            ));
            let member =
                LineRef::parse("@batch=reply :irc.example 401 papo missing :No such nick").unwrap();
            let _ = batches.handle(now, &member, usize::MAX, false, pending);
            let close = LineRef::parse("BATCH -reply").unwrap();
            let BatchAction::Complete(events) =
                batches.handle(now, &close, usize::MAX, false, pending)
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
