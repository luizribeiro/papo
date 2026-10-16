//! Tokio-based IRC client orchestration, transports, and ergonomic handles.
//!
//! TLS uses ring when both `tls-ring` and `tls-aws-lc` are enabled.

mod address;
mod builder;
mod client;
mod driver;
mod entropy;
mod error;
mod event;
mod ext;
mod history;
mod metadata;
mod reconnect;
mod requests;
mod state;

#[cfg(feature = "bot")]
pub mod bot;

pub use address::AddressError;
pub use builder::Builder;
pub use client::{Client, PushKeys, Sent, WebPush};
pub use driver::{Driver, Events};
pub use entropy::OsEntropy;
pub use error::Error;
pub use event::{Event, MessageEvent, MessageKind, Origin};
pub use ext::accounts::AccountRegistration;
pub use ext::persistence::Persistence;
pub use history::{History, HistoryPage, HistoryReference, HistoryRequest};
pub use metadata::Metadata;
pub use papo_core::WebIrc;
pub use papo_core::{Account, Capabilities, Channel, Member, Response, ServerError, User};
pub use papo_core::{PersistenceSetting, PersistenceStatus, Secret};
pub use papo_core::{RateLimit, Sasl};
pub use papo_proto::Typing;
pub use reconnect::Reconnect;
pub use requests::{ChannelInfo, ListEntry, ListFilter, WhoEntry, Whois};
pub use state::State;

/// Strict Transport Security policy stores.
pub mod sts;

/// TCP and TLS connections to IRC servers.
pub mod transport;
