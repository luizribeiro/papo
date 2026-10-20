use super::{
    Command, Event, HashMap, LineRef, MAX_METADATA_TARGETS, MetadataValue, NameKey, Numeric,
    Outbox, State, StateLimit, metadata_update,
};

impl State {
    pub(super) fn handle_metadata(
        &mut self,
        message: &LineRef<'_>,
        max_keys: usize,
        metadata_subs_batch: bool,
        outbox: &mut Outbox,
    ) -> bool {
        if message.command() == Command::Numeric(Numeric::RPL_METADATASYNCLATER) {
            let Some(target) = message.param(1) else {
                return false;
            };
            let retry_after = match message.param(2) {
                Some(seconds) => {
                    let Ok(seconds) = seconds.parse() else {
                        return false;
                    };
                    Some(std::time::Duration::from_secs(seconds))
                }
                None => None,
            };
            outbox.event(Event::MetadataSyncLater {
                target: target.to_owned(),
                retry_after,
            });
            return true;
        }
        match message.command() {
            Command::Numeric(Numeric::RPL_METADATASUBOK) => {
                for key in message.params().skip(1) {
                    if !self.metadata_subscriptions.iter().any(|item| item == key) {
                        self.metadata_subscriptions.push(key.to_owned());
                    }
                }
                return true;
            }
            Command::Numeric(Numeric::RPL_METADATAUNSUBOK) => {
                self.metadata_subscriptions
                    .retain(|key| !message.params().skip(1).any(|item| item == key));
                return true;
            }
            Command::Numeric(Numeric::RPL_METADATASUBS) => {
                if !metadata_subs_batch {
                    self.metadata_subscriptions.clear();
                }
                for key in message.params().skip(1) {
                    if !self.metadata_subscriptions.iter().any(|item| item == key) {
                        self.metadata_subscriptions.push(key.to_owned());
                    }
                }
                return true;
            }
            _ => {}
        }
        let Some((target, key, visibility, value)) = metadata_update(message) else {
            return false;
        };
        self.update_metadata(target, key, visibility, value, max_keys, outbox);
        true
    }

    pub(super) fn update_metadata(
        &mut self,
        target: &str,
        key: &str,
        visibility: &str,
        value: Option<&str>,
        max_keys: usize,
        outbox: &mut Outbox,
    ) {
        let target_key = NameKey::new(target, self.casemapping);
        if let Some(value) = value {
            if !self.metadata.contains_key(&target_key)
                && self.metadata.len() >= MAX_METADATA_TARGETS
            {
                self.warn_limit(StateLimit::MetadataTargets, outbox);
                return;
            }
            let values = self.metadata.get(&target_key);
            if values.is_none_or(|values| !values.contains_key(key))
                && values.map_or(0, HashMap::len) >= max_keys
            {
                self.warn_limit(StateLimit::MetadataKeys, outbox);
                return;
            }
            self.metadata.entry(target_key).or_default().insert(
                key.to_owned(),
                MetadataValue {
                    value: value.to_owned(),
                    visibility: visibility.to_owned(),
                },
            );
        } else if let Some(values) = self.metadata.get_mut(&target_key) {
            _ = values.remove(key);
            if values.is_empty() {
                self.metadata.remove(&target_key);
            }
        }
        outbox.event(Event::MetadataChanged {
            target: target.to_owned(),
            key: key.to_owned(),
            value: value.map(str::to_owned),
            visibility: visibility.to_owned(),
        });
    }
}
