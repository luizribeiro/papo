//! Shared support for tests against a live server.
#![allow(dead_code)]

use std::{future::Future, time::Duration};

/// Constructs a typed nickname for pre-connect test input.
pub fn nick(value: impl AsRef<str>) -> papo::Nick {
    papo::__testing::nickname(papo::Casemapping::default(), value.as_ref())
}

/// Constructs a typed channel name for test input.
pub fn channel(value: impl AsRef<str>) -> papo::ChannelName {
    papo::__testing::channel_name(papo::Casemapping::default(), value.as_ref())
}

/// Constructs a typed name mask for test input.
pub fn mask(value: impl AsRef<str>) -> papo::Mask {
    papo::__testing::mask(papo::Casemapping::default(), value.as_ref())
}

/// Constructs a typed account name for test input.
pub fn account(value: impl AsRef<str>) -> papo::AccountName {
    papo::__testing::account_name(papo::Casemapping::default(), value.as_ref())
}

/// Fails a live test when an operation does not finish promptly.
pub async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("live test operation exceeded twenty seconds")
}
