use std::{future::Future, pin::Pin, time::SystemTime};

use tokio::io::{AsyncRead, AsyncWrite};
use web_time::Instant;

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
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
