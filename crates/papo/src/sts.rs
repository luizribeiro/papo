//! Strict Transport Security policy stores.
//!
//! Clients cache policies in memory by default. Implement [`StsStore`](crate::sts::StsStore) for a
//! custom cache.

#[doc(inline)]
pub use papo_core::sts::{StsPolicy, StsStore};

pub(crate) fn normalize_host(host: &str) -> String {
    host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase()
}
