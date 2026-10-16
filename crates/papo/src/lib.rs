//! Tokio-based IRC client orchestration, transports, and ergonomic handles.
//!
//! TLS uses ring when both `tls-ring` and `tls-aws-lc` are enabled.

mod address;
mod builder;
mod driver;
mod entropy;
mod error;
mod event;
mod history;
mod reconnect;
mod requests;
mod state;

#[cfg(feature = "bot")]
pub mod bot;

pub use address::AddressError;
pub use builder::Builder;
pub use driver::{Client, Driver, Events, Sent};
pub use entropy::OsEntropy;
pub use error::Error;
pub use event::{Event, MessageEvent, MessageKind, Origin};
pub use history::{History, HistoryPage, HistoryReference, HistoryRequest};
pub use papo_core::{Account, Capabilities, Channel, Member, Response, ServerError, User};
pub use papo_core::{RateLimit, Sasl};
pub use papo_proto::Typing;
pub use reconnect::Reconnect;
pub use requests::{ChannelInfo, ListEntry, ListFilter, WhoEntry, Whois};
pub use state::State;

/// Strict Transport Security policy stores.
pub mod sts;

/// TCP and TLS connections to IRC servers.
pub mod transport;
