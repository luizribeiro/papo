use std::io;

use tokio::io::{AsyncReadExt, DuplexStream};

const INTERRUPTED: &str = "wire fixture I/O was interrupted";

/// Owns the server side of a scripted connection.
pub struct WireIo(Option<DuplexStream>);

impl std::fmt::Debug for WireIo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::opaque_debug("WireIo", formatter)
    }
}

impl WireIo {
    /// Wraps an in-memory connection for scripted I/O.
    #[must_use]
    pub const fn new(stream: DuplexStream) -> Self {
        Self(Some(stream))
    }

    /// Reads one byte from the connection.
    ///
    /// # Cancellation
    ///
    /// Dropping a pending read leaves the fixture usable because no byte was consumed.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error, including EOF.
    ///
    /// # Panics
    ///
    /// Panics if a prior operation poisoned the fixture.
    pub async fn read_byte(&mut self) -> io::Result<u8> {
        self.0.as_mut().expect(INTERRUPTED).read_u8().await
    }

    /// Runs an operation with exclusive access to the connection.
    ///
    /// # Cancellation
    ///
    /// Dropping a pending operation closes the connection and poisons the fixture.
    ///
    /// # Panics
    ///
    /// Panics if a prior operation poisoned the fixture.
    pub async fn run<T>(&mut self, op: impl std::ops::AsyncFnOnce(&mut DuplexStream) -> T) -> T {
        let mut stream = self.0.take().expect(INTERRUPTED);
        let result = op(&mut stream).await;
        self.0 = Some(stream);
        result
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        task::{Context, Waker},
    };

    use tokio::io::AsyncWriteExt as _;

    use super::*;

    #[tokio::test]
    async fn pending_byte_read_leaves_the_fixture_usable() {
        let (server, mut client) = tokio::io::duplex(64);
        let mut server = WireIo::new(server);
        let mut read = Box::pin(server.read_byte());
        assert!(
            read.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(read);

        client
            .write_all(b"PASS local-test-password\r\n")
            .await
            .unwrap();
        assert_eq!(format!("{server:?}"), "WireIo { .. }");
        assert_eq!(server.read_byte().await.unwrap(), b'P');
        assert_eq!(
            server.run(async |io| io.read_u8().await).await.unwrap(),
            b'A'
        );
        assert_eq!(server.read_byte().await.unwrap(), b'S');
    }
}
