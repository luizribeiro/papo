//! Shared support for tests against a live server.
#![allow(dead_code)]

use std::{future::Future, time::Duration};

fn state() -> std::sync::Arc<papo::State> {
    let (client, _events, _driver) = papo::Client::builder("ircs://offline.test").build();
    client.state()
}

/// Constructs a typed nickname for pre-connect test input.
pub fn nick(value: impl AsRef<str>) -> papo::Nick {
    state().nickname(value)
}

/// Constructs a typed channel name for test input.
pub fn channel(value: impl AsRef<str>) -> papo::ChannelName {
    state().channel_name(value)
}

/// Constructs a typed name mask for test input.
pub fn mask(value: impl AsRef<str>) -> papo::Mask {
    state().mask(value)
}

/// Constructs a typed account name for test input.
pub fn account(value: impl AsRef<str>) -> papo::AccountName {
    state().account_name(value)
}

/// Fails a live test when an operation does not finish promptly.
pub async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("live test operation exceeded twenty seconds")
}
