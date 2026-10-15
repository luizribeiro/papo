//! Tokio-based IRC client orchestration, transports, and ergonomic handles.
//!
//! TLS uses ring when both `tls-ring` and `tls-aws-lc` are enabled.

mod address;
mod builder;
mod driver;
mod entropy;
mod error;
mod event;
mod reconnect;
mod state;

pub use address::AddressError;
pub use builder::Builder;
pub use driver::{Client, Driver, Events};
pub use entropy::OsEntropy;
pub use error::Error;
pub use event::{Event, MessageEvent, MessageKind, Origin};
pub use papo_core::{RateLimit, Sasl};
pub use reconnect::Reconnect;

/// Strict Transport Security policy stores.
pub mod sts;

/// TCP and TLS connections to IRC servers.
pub mod transport;
