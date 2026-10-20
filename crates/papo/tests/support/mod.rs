//! Shared support for tests against a live server.
#![allow(dead_code)]

use std::{future::Future, time::Duration};

/// Constructs a typed nickname for pre-registration test input.
pub fn nick(value: impl AsRef<str>) -> papo::Nick {
    papo::Casemapping::default().nick(value.as_ref())
}

/// Constructs a typed channel name for test input.
pub fn channel(value: impl AsRef<str>) -> papo::ChannelName {
    papo::Casemapping::default().channel(value.as_ref())
}

/// Constructs a typed name mask for test input.
pub fn mask(value: impl AsRef<str>) -> papo::Mask {
    papo::Casemapping::default().mask(value.as_ref())
}

/// Constructs a typed account name for test input.
pub fn account(value: impl AsRef<str>) -> papo::AccountName {
    papo::Casemapping::default().account(value.as_ref())
}

/// Fails a live test when an operation does not finish promptly.
pub async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("live test operation exceeded twenty seconds")
}
