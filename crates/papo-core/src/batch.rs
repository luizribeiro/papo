use std::collections::HashMap;

use papo_proto::MessageRef;

#[derive(Debug, Default)]
pub(crate) struct Batches {
    open: HashMap<String, OpenBatch>,
}

#[derive(Debug)]
struct OpenBatch {
    kind: String,
    parent: Option<String>,
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
    Dispatch(DispatchContext),
}

impl Batches {
    pub(crate) fn reset(&mut self) {
        self.open.clear();
    }

    pub(crate) fn handle(&mut self, message: &MessageRef<'_>) -> BatchAction {
        if !message.command_is("BATCH") {
            return BatchAction::Dispatch(self.context(message));
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
        let parent = message.tag("batch").map(|value| value.into_owned());
        if !valid_reference(reference)
            || self.open.contains_key(reference)
            || parent
                .as_ref()
                .is_some_and(|parent| !self.open.contains_key(parent))
        {
            return BatchAction::Dispatch(self.context(message));
        }
        self.open.insert(
            reference.to_owned(),
            OpenBatch {
                kind: kind.to_owned(),
                parent,
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
        self.open.remove(reference);
        BatchAction::Consumed
    }

    fn context(&self, message: &MessageRef<'_>) -> DispatchContext {
        let batch_type = message
            .tag("batch")
            .and_then(|reference| self.open.get(reference.as_ref()))
            .map(|batch| batch.kind.clone());
        DispatchContext { batch_type }
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
    use super::{BatchAction, Batches};
    use papo_proto::MessageRef;

    fn handle(batches: &mut Batches, line: &str) -> Option<Option<String>> {
        let message = MessageRef::parse(line).unwrap();
        match batches.handle(&message) {
            BatchAction::Consumed => None,
            BatchAction::Dispatch(context) => Some(context.batch_type().map(str::to_owned)),
        }
    }

    #[test]
    fn resolves_nested_and_interleaved_batch_types() {
        let mut batches = Batches::default();
        assert_eq!(handle(&mut batches, "BATCH +outer vendor/sync"), None);
        assert_eq!(handle(&mut batches, "BATCH +other vendor/other"), None);
        assert_eq!(handle(&mut batches, "@batch=outer BATCH +i nested"), None);
        assert_eq!(
            handle(&mut batches, "@batch=i :irc.example NOTICE papo :nested"),
            Some(Some("nested".into()))
        );
        assert_eq!(
            handle(&mut batches, "@batch=other :irc.example NOTICE papo :other"),
            Some(Some("vendor/other".into()))
        );
    }

    #[test]
    fn passes_through_invalid_envelopes() {
        let mut batches = Batches::default();
        assert_eq!(handle(&mut batches, "BATCH +bad_ref sync"), Some(None));
        assert_eq!(handle(&mut batches, "BATCH -missing"), Some(None));
    }
}
