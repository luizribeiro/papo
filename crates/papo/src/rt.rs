use std::{
    future::Future,
    time::{Instant, SystemTime},
};

pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

pub(crate) async fn sleep_until(deadline: Instant) {
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

pub(crate) fn now_wall() -> SystemTime {
    SystemTime::now()
}

pub(crate) fn now_mono() -> Instant {
    tokio::time::Instant::now().into_std()
}
