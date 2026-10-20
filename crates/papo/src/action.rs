use std::{future::Future, pin::Pin, task};

use crate::Error;

/// An owned, lazy IRC request.
///
/// The request is not submitted to the client driver until this future is
/// first polled. Dropping it after polling abandons the wait but does not
/// cancel a command already sent to the server.
///
/// This future is [`Send`] on every supported target and deliberately not
/// [`Sync`]; polling requires exclusive access.
#[must_use = "requests do nothing unless awaited"]
pub struct RequestFuture<T> {
    inner: Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'static>>,
}

impl<T> RequestFuture<T> {
    pub(crate) fn new(future: impl Future<Output = Result<T, Error>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(future),
        }
    }
}

impl<T> Future for RequestFuture<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, context: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        self.inner.as_mut().poll(context)
    }
}
