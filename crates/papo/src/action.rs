use std::{future::Future, pin::Pin, task};

use crate::Error;

/// An owned, lazy IRC request.
///
/// The request is not submitted to the client driver until this future is
/// first polled. Dropping it after polling abandons the wait but does not
/// cancel a command already sent to the server.
///
/// `Ok` means all request lines were written and the expected reply arrived where one exists.
/// [`Error::Timeout`] means the lines were written but no reply arrived before
/// the deadline. [`Error::Disconnected`] means they were not written, while
/// [`Error::Interrupted`] means transmission began but the reply was still
/// pending when the connection ended.
///
/// # Retrying after a timeout
///
/// A timeout is uncertain delivery: every line was written, but papo did not observe the reply.
/// Use this table before retrying.
///
/// | Request | Safe to retry automatically? | Why |
/// |---|---|---|
/// | Read-only queries: `whois`, `who`, `list`, `list_modes`, `history`, read-marker fetch, metadata get/list/sync, bouncer network listing, persistence get, `ping`, and direct-recipient discovery | Yes | Repeating the query does not repeat a remote state change. The result may reflect newer state. |
/// | Chat sends (`privmsg`, `notice`, actions and CTCP), reactions, redaction, and read-marker updates | No | The first request may already be visible or applied. A retry can duplicate it. This includes every line of a split or multiline send. |
/// | `KICK`, `MODE`, `kickban`, join/part/invite, topic/nick/away/realname changes, and channel rename | No | The server may have changed state even though its confirmation was lost. Re-read state or query the server before deciding what to do. |
/// | Account, operator, metadata, monitor, bouncer, persistence, and web-push mutations | No | Retrying can repeat an action or apply it to state that changed after the first request. |
/// | [`Client::request_raw`](crate::Client::request_raw) and extension requests not listed above | It depends | Apply the IRC command's own idempotency and correlation rules. Treat an unknown command as unsafe. |
///
/// Dropping a polled future has the same uncertainty: it stops waiting, not work already accepted
/// by the driver. Composite operations such as `kickban`, batched mode changes, and multiline
/// redaction continue after the future is dropped.
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

impl<T> std::fmt::Debug for RequestFuture<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("RequestFuture", formatter)
    }
}

impl<T> Future for RequestFuture<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, context: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        self.inner.as_mut().poll(context)
    }
}
