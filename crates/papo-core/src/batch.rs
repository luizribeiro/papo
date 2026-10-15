use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use papo_proto::{Message, MessageRef, time};

use crate::{BatchAbandonReason, Config, HistoryMessage, HistoryPage};

#[derive(Debug)]
pub(crate) struct Batches {
    open: HashMap<String, OpenBatch>,
    max_open: usize,
    max_lines: usize,
    timeout: Duration,
}

#[derive(Debug)]
struct OpenBatch {
    opening: Message,
    kind: String,
    label: Option<String>,
    parent: Option<String>,
    params: Vec<String>,
    end: bool,
    messages: Vec<Message>,
    completed: Vec<BatchEvent>,
    lines: usize,
    bytes: usize,
    opened: Instant,
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
        messages: Vec<Message>,
        batch_type: String,
        history: Option<HistoryPage>,
        history_targets: Option<Vec<(String, std::time::SystemTime)>>,
    },
    History {
        target: String,
        messages: Vec<HistoryMessage>,
        end: bool,
    },
    HistoryTargets(Vec<(String, std::time::SystemTime)>),
    Netsplit {
        servers: [String; 2],
        quits: Vec<Message>,
    },
    Netjoin {
        servers: [String; 2],
        joins: Vec<Message>,
    },
    Abandoned {
        reference: String,
        batch_type: String,
        reason: BatchAbandonReason,
    },
    Message {
        message: Message,
        batch_type: Option<String>,
    },
    Multiline(Message),
}

impl Batches {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            open: HashMap::new(),
            max_open: config.max_open_batches,
            max_lines: config.max_batch_lines,
            timeout: config.batch_timeout,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.open.clear();
    }

    pub(crate) fn handle(
        &mut self,
        now: Instant,
        message: &MessageRef<'_>,
        multiline_max_bytes: usize,
    ) -> BatchAction {
        if !message.command_is("BATCH") {
            return self.member(message, multiline_max_bytes);
        }
        let Some(reference) = message.param(0) else {
            return BatchAction::Dispatch(self.context(message));
        };
        if let Some(reference) = reference.strip_prefix('+') {
            return self.open(now, message, reference);
        }
        if let Some(reference) = reference.strip_prefix('-') {
            return self.close(message, reference);
        }
        BatchAction::Dispatch(self.context(message))
    }

    fn open(&mut self, now: Instant, message: &MessageRef<'_>, reference: &str) -> BatchAction {
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
                label: message.tag("label").map(std::borrow::Cow::into_owned),
                parent,
                params: message.params().skip(2).map(str::to_owned).collect(),
                end: message
                    .tags()
                    .any(|(key, _)| key == "draft/chathistory-end"),
                messages: Vec::new(),
                completed: Vec::new(),
                lines: 0,
                bytes: 0,
                opened: now,
            },
        );
        BatchAction::Consumed
    }

    fn close(&mut self, message: &MessageRef<'_>, reference: &str) -> BatchAction {
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
        let mut completed = std::mem::take(&mut batch.completed);
        let response_messages = batch.label.as_ref().map(|_| batch.messages.clone());
        if batch.is_typed_aggregate() && !self.has_typed_aggregate_ancestor(batch.parent.as_deref())
        {
            completed.extend(complete(&mut batch));
        }
        if let Some(label) = batch.label.take() {
            let history = completed.iter().find_map(history_from_event);
            let history_targets = completed.iter().find_map(|event| match event {
                BatchEvent::HistoryTargets(targets) => Some(targets.clone()),
                _ => None,
            });
            completed.push(BatchEvent::Response {
                label,
                messages: response_messages.expect("label captured response messages"),
                batch_type: batch.kind.clone(),
                history,
                history_targets,
            });
        }
        if let Some(parent) = batch.parent {
            self.open
                .get_mut(&parent)
                .expect("validated batch parent remains open")
                .completed
                .append(&mut completed);
            BatchAction::Consumed
        } else {
            BatchAction::Complete(completed)
        }
    }

    fn member(&mut self, message: &MessageRef<'_>, multiline_max_bytes: usize) -> BatchAction {
        let context = self.context(message);
        let Some(reference) = message.tag("batch") else {
            return BatchAction::Dispatch(context);
        };
        let ancestors = self.ancestors(reference.as_ref());
        for reference in &ancestors {
            let batch = self.open.get_mut(reference).expect("resolved ancestor");
            batch.lines = batch.lines.saturating_add(1);
            if batch.kind == "draft/multiline" {
                let concat = message
                    .tags()
                    .any(|(key, _)| key == "draft/multiline-concat");
                batch.bytes = batch.bytes.saturating_add(
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
                    || (self.open[*reference].kind == "draft/multiline"
                        && self.open[*reference].bytes > multiline_max_bytes)
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
                let fallback = BatchEvent::Message {
                    message: message.to_owned(),
                    batch_type: context.batch_type,
                };
                events.insert(events.len() - 1, fallback);
            }
            return BatchAction::Complete(events);
        }
        if aggregates.is_empty() {
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
                batch
                    .opened
                    .checked_add(self.timeout)
                    .is_some_and(|deadline| now >= deadline)
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
            .filter_map(|batch| batch.opened.checked_add(self.timeout))
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
        let types: HashMap<_, _> = references
            .iter()
            .map(|candidate| (candidate.clone(), self.open[candidate].kind.clone()))
            .collect();
        let batch_type = types[reference].clone();
        let mut messages = Vec::new();
        for candidate in references {
            let batch = self.open.remove(&candidate).expect("selected open batch");
            messages.extend(batch.messages);
            for event in batch.completed {
                collect_messages(event, &mut messages);
            }
        }
        let mut events: Vec<_> = messages
            .into_iter()
            .map(|message| BatchEvent::Message {
                batch_type: message
                    .as_ref()
                    .tag("batch")
                    .and_then(|reference| types.get(reference.as_ref()))
                    .cloned(),
                message,
            })
            .collect();
        events.push(BatchEvent::Abandoned {
            reference: reference.to_owned(),
            batch_type,
            reason,
        });
        events
    }

    fn aggregate_references(&self, reference: &str) -> Vec<String> {
        let ancestors = self.ancestors(reference);
        let mut aggregates = Vec::new();
        if let Some(reference) = ancestors
            .iter()
            .rev()
            .find(|reference| is_aggregate(&self.open[*reference].kind))
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

    fn context(&self, message: &MessageRef<'_>) -> DispatchContext {
        let batch_type = message
            .tag("batch")
            .and_then(|reference| self.open.get(reference.as_ref()))
            .map(|batch| batch.kind.clone());
        DispatchContext { batch_type }
    }
}

fn is_history(kind: &str) -> bool {
    matches!(
        kind,
        "chathistory" | "draft/chathistory" | "draft/chathistory-targets" | "draft/event-playback"
    )
}

fn is_aggregate(kind: &str) -> bool {
    is_history(kind) || matches!(kind, "netsplit" | "netjoin" | "draft/multiline")
}

impl OpenBatch {
    fn is_typed_aggregate(&self) -> bool {
        is_aggregate(&self.kind)
    }
}

fn complete(batch: &mut OpenBatch) -> Vec<BatchEvent> {
    if batch.kind == "draft/multiline" {
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
            return vec![BatchEvent::History {
                target: batch.params.first().cloned().unwrap_or_default(),
                messages: history_messages(std::mem::take(&mut batch.messages)),
                end: batch.end,
            }];
        }
    };
    let (members, others): (Vec<_>, Vec<_>) = std::mem::take(&mut batch.messages)
        .into_iter()
        .partition(|message| message.as_ref().command_is(expected_command));
    let aggregate = if expected_command == "QUIT" {
        BatchEvent::Netsplit {
            servers: servers(),
            quits: members,
        }
    } else {
        BatchEvent::Netjoin {
            servers: servers(),
            joins: members,
        }
    };
    std::iter::once(aggregate)
        .chain(others.into_iter().map(|message| BatchEvent::Message {
            message,
            batch_type: Some(batch.kind.clone()),
        }))
        .collect()
}

fn multiline(batch: &mut OpenBatch) -> Option<Message> {
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
    let mut message = Message::new(command, [target, &text]);
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

fn history_from_event(event: &BatchEvent) -> Option<HistoryPage> {
    let BatchEvent::History {
        target,
        messages,
        end,
    } = event
    else {
        return None;
    };
    Some(HistoryPage {
        target: target.clone(),
        messages: messages.clone(),
        end: *end,
    })
}

fn collect_messages(event: BatchEvent, messages: &mut Vec<Message>) {
    match event {
        BatchEvent::Response {
            messages: batch, ..
        } => messages.extend(batch),
        BatchEvent::History {
            messages: batch, ..
        } => {
            messages.extend(batch.into_iter().map(|item| item.message));
        }
        BatchEvent::Netsplit { quits, .. } => messages.extend(quits),
        BatchEvent::Netjoin { joins, .. } => messages.extend(joins),
        BatchEvent::Message { message, .. } | BatchEvent::Multiline(message) => {
            messages.push(message);
        }
        BatchEvent::HistoryTargets(_) | BatchEvent::Abandoned { .. } => {}
    }
}

fn history_messages(messages: Vec<Message>) -> Vec<HistoryMessage> {
    let mut msgids = HashSet::new();
    messages
        .into_iter()
        .filter_map(|message| {
            let parsed = message.as_ref();
            let msgid = parsed.tag("msgid").map(std::borrow::Cow::into_owned);
            let context = parsed
                .tags()
                .any(|(key, _)| key == "draft/chathistory-context");
            msgid
                .is_none_or(|msgid| msgids.insert(msgid))
                .then_some(HistoryMessage { message, context })
        })
        .collect()
}

fn valid_reference(reference: &str) -> bool {
    !reference.is_empty()
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::{BatchAction, BatchEvent, Batches};
    use crate::Config;
    use papo_proto::MessageRef;
    use std::time::Instant;

    fn handle(batches: &mut Batches, line: &str) -> (bool, Option<String>) {
        let message = MessageRef::parse(line).unwrap();
        match batches.handle(Instant::now(), &message, usize::MAX) {
            BatchAction::Consumed | BatchAction::Complete(_) => (false, None),
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
            (true, Some("nested".into()))
        );
        assert_eq!(
            handle(&mut batches, "@batch=other :irc.example NOTICE papo :other"),
            (true, Some("vendor/other".into()))
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
        let close = MessageRef::parse("BATCH -h").unwrap();
        let BatchAction::Complete(events) = batches.handle(Instant::now(), &close, usize::MAX)
        else {
            panic!("history did not complete")
        };
        let BatchEvent::History {
            target, messages, ..
        } = &events[0]
        else {
            panic!("expected history")
        };
        assert_eq!(target, "#papo");
        assert_eq!(messages[0].as_ref().param(1), Some("hello"));
    }
}
