use std::{io, path::PathBuf};

use papo_proto::BuildError;
use thiserror::Error;

use crate::{AddressError, transport::ConnectError};

/// An error produced by the asynchronous IRC client.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The configured server address is invalid.
    #[error("invalid server address: {0}")]
    Address(#[from] AddressError),
    /// Reading a configured file failed.
    #[error("failed to read {path}: {source}")]
    ReadFile {
        /// The path that could not be read.
        path: PathBuf,
        /// The filesystem error.
        #[source]
        source: io::Error,
    },
    /// Reading from or writing to the connection failed.
    #[error("connection I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Opening the connection failed.
    #[error("connection failed: {0}")]
    Connect(#[from] ConnectError),
    /// A stored STS policy conflicts with explicitly forced plaintext.
    #[error("STS requires TLS for {host}, but plaintext was explicitly selected")]
    StsRequiresTls {
        /// The hostname protected by the stored policy.
        host: String,
    },
    /// A server requested consecutive STS upgrades without registering.
    #[error("repeated STS upgrade for {host} without registration")]
    StsUpgradeLoop {
        /// The hostname whose server repeated the upgrade.
        host: String,
    },
    /// The requested transport is not available in this build.
    #[error("unsupported transport: {0}")]
    Unsupported(&'static str),
    /// An outbound IRC message is not valid on the wire.
    #[error("invalid IRC message: {0}")]
    Build(#[from] BuildError),
    /// The connection already has 256 requests awaiting replies.
    #[error("too many IRC requests are awaiting replies")]
    RequestLimit,
    /// Registration ended before the server welcomed the client.
    #[error("IRC registration failed: {0}")]
    Registration(String),
    /// The server disconnected before the requested operation completed.
    #[error("the IRC connection closed")]
    Disconnected,
    /// An operation did not complete before its deadline.
    #[error("the IRC operation timed out")]
    Timeout,
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::Error;

    #[test]
    fn io_error_keeps_its_source() {
        let error = Error::from(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        ));
        assert_eq!(
            error.to_string(),
            "connection I/O failed: connection reset by peer"
        );
    }

    #[test]
    fn terminal_errors_have_distinct_messages() {
        assert_eq!(Error::Disconnected.to_string(), "the IRC connection closed");
        assert_eq!(Error::Timeout.to_string(), "the IRC operation timed out");
    }
}
