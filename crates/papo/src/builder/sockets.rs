use papo_core::sts::StsStore;

use super::Builder;

impl Builder {
    /// Sets the store used for IRC Strict Transport Security policies.
    pub fn sts_store(mut self, store: impl StsStore + 'static) -> Self {
        self.sts_store = std::sync::Arc::new(store);
        self
    }
}
