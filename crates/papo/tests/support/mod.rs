//! Shared support for tests against a live server.
#![allow(dead_code)]

use std::{future::Future, time::Duration};

/// Constructs a typed nickname for pre-connect test input.
pub fn nick(value: impl AsRef<str>) -> papo::Nick {
    papo_testing::nick(papo::Casemapping::default(), value)
}

/// Constructs a typed channel name for test input.
pub fn channel(value: impl AsRef<str>) -> papo::ChannelName {
    papo_testing::channel(papo::Casemapping::default(), value)
}

/// Constructs a typed name mask for test input.
pub fn mask(value: impl AsRef<str>) -> papo::Mask {
    papo_testing::mask(papo::Casemapping::default(), value)
}

/// Constructs a typed account name for test input.
pub fn account(value: impl AsRef<str>) -> papo::AccountName {
    papo_testing::account(papo::Casemapping::default(), value)
}

/// Fails a live test when an operation does not finish promptly.
pub async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("live test operation exceeded twenty seconds")
}
