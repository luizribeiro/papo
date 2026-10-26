use papo_proto::LineRef;

use crate::{BouncerNetwork, Event, Outbox, StateLimit, bouncer::NetworkUpdate};

use super::State;
#[cfg(test)]
use super::StateSnapshot;

const MAX_BOUNCER_NETWORKS: usize = 10_000;

impl State {
    #[cfg(test)]
    pub(super) fn bouncer_snapshot(&self, message: &LineRef<'_>) -> Option<StateSnapshot> {
        let update = NetworkUpdate::parse(message)?;
        let network = self.bouncer_networks.get(&update.id).cloned();
        Some(StateSnapshot::Bouncer {
            id: update.id,
            network,
        })
    }

    pub(super) fn handle_bouncer(&mut self, message: &LineRef<'_>, outbox: &mut Outbox) -> bool {
        let Some(update) = NetworkUpdate::parse(message) else {
            return false;
        };
        if update.removed {
            self.bouncer_networks.remove(&update.id);
            outbox.event(Event::BouncerNetworkChanged {
                id: update.id,
                network: None,
            });
            return true;
        }
        if !self.bouncer_networks.contains_key(&update.id)
            && self.bouncer_networks.len() >= MAX_BOUNCER_NETWORKS
        {
            self.warn_limit(StateLimit::BouncerNetworks, outbox);
            return true;
        }
        let network = self
            .bouncer_networks
            .entry(update.id.clone())
            .or_insert_with(|| BouncerNetwork::empty(update.id.clone()));
        network.apply(update.attributes);
        outbox.event(Event::BouncerNetworkChanged {
            id: update.id,
            network: Some(network.clone()),
        });
        true
    }
}
