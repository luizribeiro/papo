use std::{future::Future, time::SystemTime};
use web_time::Instant;

pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

#[cfg(not(browser))]
pub(crate) async fn sleep_until(deadline: Instant) {
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

#[cfg(browser)]
pub(crate) async fn sleep_until(_deadline: Instant) {
    // Browser timers arrive with the transport runtime; placeholder connections fail first.
    std::future::pending().await
}

pub(crate) fn now_wall() -> SystemTime {
    SystemTime::now()
}

#[cfg(not(browser))]
pub(crate) fn now_mono() -> Instant {
    tokio::time::Instant::now().into_std()
}

#[cfg(browser)]
pub(crate) fn now_mono() -> Instant {
    Instant::now()
}
