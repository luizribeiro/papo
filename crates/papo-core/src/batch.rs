use std::collections::HashMap;

use papo_proto::{Message, MessageRef};

use crate::{BatchAbandonReason, Config};

#[derive(Debug)]
pub(crate) struct Batches {
    open: HashMap<String, OpenBatch>,
    max_open: usize,
    max_lines: usize,
}

#[derive(Debug)]
struct OpenBatch {
    kind: String,
    parent: Option<String>,
    params: Vec<String>,
    end: bool,
    messages: Vec<Message>,
    completed: Vec<BatchEvent>,
    lines: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DispatchContext {
    batch_type: Option<String>,
}

impl DispatchContext {
    pub(crate) fn batch_type(&self) -> Option<&str> {
        self.batch_type.as_deref()
    }
}

pub(crate) enum BatchAction {
    Consumed,
    Complete(Vec<BatchEvent>),
    Dispatch(DispatchContext),
}

#[derive(Debug)]
pub(crate) enum BatchEvent {
    History {
        target: String,
        messages: Vec<Message>,
        end: bool,
    },
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
}

impl Batches {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            open: HashMap::new(),
            max_open: config.max_open_batches,
            max_lines: config.max_batch_lines,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.open.clear();
    }

    pub(crate) fn handle(&mut self, message: &MessageRef<'_>) -> BatchAction {
        if !message.command_is("BATCH") {
            return self.member(message);
        }
        let Some(reference) = message.param(0) else {
            return BatchAction::Dispatch(self.context(message));
        };
        if let Some(reference) = reference.strip_prefix('+') {
            return self.open(message, reference);
        }
        if let Some(reference) = reference.strip_prefix('-') {
            return self.close(message, reference);
        }
        BatchAction::Dispatch(self.context(message))
    }

    fn open(&mut self, message: &MessageRef<'_>, reference: &str) -> BatchAction {
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
                kind: kind.to_owned(),
                parent,
                params: message.params().skip(2).map(str::to_owned).collect(),
                end: message
                    .tags()
                    .any(|(key, _)| key == "draft/chathistory-end"),
                messages: Vec::new(),
                completed: Vec::new(),
                lines: 0,
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
        if is_aggregate(&batch.kind) && !self.has_aggregate_ancestor(batch.parent.as_deref()) {
            completed.push(complete(&mut batch));
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

    fn member(&mut self, message: &MessageRef<'_>) -> BatchAction {
        let context = self.context(message);
        let Some(reference) = message.tag("batch") else {
            return BatchAction::Dispatch(context);
        };
        let ancestors = self.ancestors(reference.as_ref());
        for reference in &ancestors {
            let batch = self.open.get_mut(reference).expect("resolved ancestor");
            batch.lines = batch.lines.saturating_add(1);
        }
        let aggregate = message
            .tag("batch")
            .and_then(|reference| self.aggregate_reference(reference.as_ref()));
        if let Some(reference) = aggregate.as_deref() {
            self.open
                .get_mut(reference)
                .expect("resolved aggregate remains open")
                .messages
                .push(message.to_owned());
        }
        if let Some(reference) = ancestors
            .iter()
            .rev()
            .find(|reference| self.open[*reference].lines > self.max_lines)
            .cloned()
        {
            let mut events = self.abandon(&reference, BatchAbandonReason::LineLimit);
            if aggregate.is_none() {
                let fallback = BatchEvent::Message {
                    message: message.to_owned(),
                    batch_type: context.batch_type,
                };
                events.insert(events.len() - 1, fallback);
            }
            return BatchAction::Complete(events);
        }
        if aggregate.is_some() {
            BatchAction::Consumed
        } else {
            BatchAction::Dispatch(context)
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

    fn aggregate_reference(&self, reference: &str) -> Option<String> {
        let mut aggregate = None;
        let mut current = Some(reference);
        while let Some(reference) = current {
            let batch = self.open.get(reference)?;
            if is_aggregate(&batch.kind) {
                aggregate = Some(reference.to_owned());
            }
            current = batch.parent.as_deref();
        }
        aggregate
    }

    fn has_aggregate_ancestor<'a>(&'a self, mut parent: Option<&'a str>) -> bool {
        while let Some(reference) = parent {
            let Some(batch) = self.open.get(reference) else {
                return false;
            };
            if is_aggregate(&batch.kind) {
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
    is_history(kind) || matches!(kind, "netsplit" | "netjoin")
}

fn complete(batch: &mut OpenBatch) -> BatchEvent {
    let servers = || {
        [
            batch.params.first().cloned().unwrap_or_default(),
            batch.params.get(1).cloned().unwrap_or_default(),
        ]
    };
    match batch.kind.as_str() {
        "netsplit" => BatchEvent::Netsplit {
            servers: servers(),
            quits: std::mem::take(&mut batch.messages)
                .into_iter()
                .filter(|message| message.as_ref().command_is("QUIT"))
                .collect(),
        },
        "netjoin" => BatchEvent::Netjoin {
            servers: servers(),
            joins: std::mem::take(&mut batch.messages)
                .into_iter()
                .filter(|message| message.as_ref().command_is("JOIN"))
                .collect(),
        },
        _ => BatchEvent::History {
            target: batch.params.first().cloned().unwrap_or_default(),
            messages: std::mem::take(&mut batch.messages),
            end: batch.end,
        },
    }
}

fn collect_messages(event: BatchEvent, messages: &mut Vec<Message>) {
    match event {
        BatchEvent::History {
            messages: batch, ..
        } => messages.extend(batch),
        BatchEvent::Netsplit { quits, .. } => messages.extend(quits),
        BatchEvent::Netjoin { joins, .. } => messages.extend(joins),
        BatchEvent::Message { message, .. } => messages.push(message),
        BatchEvent::Abandoned { .. } => {}
    }
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

    fn handle(batches: &mut Batches, line: &str) -> (bool, Option<String>) {
        let message = MessageRef::parse(line).unwrap();
        match batches.handle(&message) {
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
        let BatchAction::Complete(events) = batches.handle(&close) else {
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
