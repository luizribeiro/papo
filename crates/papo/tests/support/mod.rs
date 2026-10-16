//! Shared support for tests against a live server.
#![allow(dead_code)]

use std::{future::Future, time::Duration};

/// Fails a live test when an operation does not finish promptly.
pub async fn within<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("live test operation exceeded twenty seconds")
}
