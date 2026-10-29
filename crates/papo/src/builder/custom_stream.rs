use super::Builder;

/// Declares whether a caller-provided stream encrypts its bytes.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum StreamSecurity {
    /// The stream carries plaintext IRC traffic.
    #[default]
    Plaintext,
    /// The stream is protected by encryption established by the caller.
    Encrypted,
}

impl Builder {
    /// Uses an already-connected asynchronous byte stream.
    ///
    /// The stream is treated as plaintext. Custom streams are driven once and do not reconnect or
    /// apply STS policies. Use [`Self::stream_with_security`] when the caller established an
    /// encrypted stream.
    pub fn stream(
        self,
        stream: impl futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        self.stream_with_security(stream, StreamSecurity::Plaintext)
    }

    /// Uses an already-connected asynchronous byte stream with explicit transport security.
    ///
    /// [`StreamSecurity::Encrypted`] is a declaration by the caller that encryption has already
    /// been established. Papo does not inspect or add encryption to a custom stream.
    pub fn stream_with_security(
        mut self,
        stream: impl futures_io::AsyncRead + futures_io::AsyncWrite + Send + Unpin + 'static,
        security: StreamSecurity,
    ) -> Self {
        self.stream = Some((Box::new(stream), security));
        self
    }
}
