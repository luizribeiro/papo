//! Strict Transport Security policy stores.

#[doc(inline)]
pub use papo_core::sts::{MemoryStsStore, StsPolicy, StsStore};

pub(crate) fn normalize_host(host: &str) -> String {
    host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase()
}
