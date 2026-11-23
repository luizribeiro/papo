use super::{
    BTreeMap, Capabilities, Command, Event, LineRef, MetadataContext, MetadataKey, MetadataValue,
    NameKey, Numeric, Outbox, State, StateLimit, Subject, metadata_update,
};
use crate::{MetadataLimits, Visibility};

// Keep the cache small even when a server allows unlimited subscriptions.
const MAX_METADATA_SUBSCRIPTIONS: usize = 512;

impl State {
    pub(super) fn remove_metadata(&mut self, target: &str) {
        self.metadata
            .remove(&NameKey::new(target, self.casemapping));
    }

    pub(super) fn rename_metadata(&mut self, old_target: &str, new_target: &str) {
        let old_key = NameKey::new(old_target, self.casemapping);
        if let Some(values) = self.metadata.remove(&old_key) {
            self.metadata
                .insert(NameKey::new(new_target, self.casemapping), values);
        }
    }

    pub(super) fn handle_metadata(
        &mut self,
        message: &LineRef<'_>,
        context: MetadataContext,
        enabled_caps: &Capabilities,
        outbox: &mut Outbox,
    ) -> bool {
        match message.command() {
            Command::Numeric(Numeric::RPL_METADATAUNSUBOK) => {
                for key in message.params().skip(1) {
                    self.metadata_subscriptions.remove(key);
                }
                return true;
            }
            // Raw and unlabelled SUB commands can have replies without a tracked request.
            Command::Numeric(Numeric::RPL_METADATASUBOK | Numeric::RPL_METADATASUBS) => {
                if message.command() == Command::Numeric(Numeric::RPL_METADATASUBS)
                    && !context.subscription_batch
                {
                    self.metadata_subscriptions.clear();
                }
                let limit = MetadataLimits::from_caps(enabled_caps)
                    .and_then(|limits| limits.max_subscriptions)
                    .unwrap_or(MAX_METADATA_SUBSCRIPTIONS)
                    .min(MAX_METADATA_SUBSCRIPTIONS);
                for key in message.params().skip(1) {
                    if self.metadata_subscriptions.len() < limit {
                        self.metadata_subscriptions.insert(MetadataKey::new(key));
                    } else if !self.metadata_subscriptions.contains(key) {
                        self.warn_limit(StateLimit::MetadataSubscriptions, outbox);
                        break;
                    }
                }
                return true;
            }
            _ => {}
        }
        let Some((target, key, visibility, value)) = metadata_update(message) else {
            return false;
        };
        self.update_metadata(target, key, visibility, value, context.max_keys, outbox);
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
        let visibility = Visibility::from_wire(visibility);
        let value = value.map(|value| MetadataValue {
            value: value.to_owned(),
            visibility,
        });
        let target_key = NameKey::new(target, self.casemapping);
        if let Some(value) = &value {
            if !self.metadata.contains_key(&target_key)
                && self.metadata.len() >= self.max_metadata_targets
            {
                self.warn_limit(StateLimit::MetadataTargets, outbox);
                return;
            }
            let values = self.metadata.get(&target_key);
            if values.is_none_or(|values| !values.contains_key(key))
                && values.map_or(0, BTreeMap::len) >= max_keys
            {
                self.warn_limit(StateLimit::MetadataKeys, outbox);
                return;
            }
            self.metadata
                .entry(target_key)
                .or_default()
                .insert(papo_proto::MetadataKey::new(key), value.clone());
        } else if let Some(values) = self.metadata.get_mut(&target_key) {
            _ = values.remove(key);
            if values.is_empty() {
                self.metadata.remove(&target_key);
            }
        }
        outbox.event(Event::MetadataChanged {
            subject: if self.isupport.is_channel(target) {
                Subject::Channel(self.casemapping.channel(target))
            } else {
                Subject::User(self.casemapping.nick(target))
            },
            subject_token: self.__reply_token_for(target),
            key: key.to_owned(),
            value,
        });
    }
}
