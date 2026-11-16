//! Real connections whose byte streams a test wraps.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::driver::{BoxConnection, Connector, open_connection};

/// A connection's byte stream after any TLS layer, as the driver reads and writes it.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Connects like [`crate::Builder::connect`], passing each opened connection through `wrap`.
///
/// Reconnects open a new connection and call `wrap` again.
///
/// # Errors
///
/// Returns the same errors as [`crate::Builder::connect`].
pub async fn connect_wrapped(
    builder: crate::Builder,
    wrap: impl Fn(Box<dyn Stream>) -> Box<dyn Stream> + Send + Sync + 'static,
) -> Result<(crate::Client, crate::Events), crate::Error> {
    let wrap = Arc::new(wrap);
    let connector: Connector = Arc::new(move |plan, progress| {
        let wrap = Arc::clone(&wrap);
        Box::pin(async move {
            let stream = open_connection(plan, progress).await?;
            Ok(Box::new(wrap(Box::new(stream))) as BoxConnection)
        })
    });
    builder.connect_through(Some(connector)).await
}
