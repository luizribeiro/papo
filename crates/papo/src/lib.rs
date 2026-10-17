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
mod io_compat;
mod reconnect;
mod requests;
mod rt;
mod state;
mod transport;

#[cfg(feature = "bot")]
pub mod bot;
pub mod cap;

pub use address::{Endpoint, IntoEndpoint, InvalidInput};
pub use builder::Builder;
pub use client::{Client, Sent};
pub use driver::{Driver, Events};
pub use entropy::OsEntropy;
pub use error::{
    ConnectError, ConnectErrorKind, DisconnectCause, Disconnected, Error, Refusal,
    RegistrationError, ServerError,
};
pub use event::{Event, MessageEvent, MessageKind, Origin};
pub use ext::accounts::{AccountRegistration, NewAccount};
pub use ext::history::{History, HistoryPage, HistoryReference, HistoryRequest};
pub use ext::metadata::Metadata;
pub use ext::persistence::Persistence;
pub use ext::webpush::{PushKeys, WebPush};
pub use papo_core::WebIrc;
pub use papo_core::{Account, Capabilities, Channel, Feature, Member, Response, User};
pub use papo_core::{PersistenceSetting, PersistenceStatus, Secret};
pub use papo_core::{RateLimit, Sasl};
pub use papo_core::{Unsupported, UnsupportedReason};
pub use papo_proto::Typing;
pub use reconnect::Reconnect;
pub use requests::{ChannelInfo, ListEntry, ListFilter, WhoEntry, Whois};
pub use state::State;

/// Strict Transport Security policy stores.
pub mod sts;
