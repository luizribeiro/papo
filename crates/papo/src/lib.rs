//! Tokio-based IRC client orchestration, transports, and ergonomic handles.
//!
//! TLS uses ring when both `tls-ring` and `tls-aws-lc` are enabled.

mod address;
mod builder;
mod client;
mod driver;
mod entropy;
mod error;
#[path = "event.rs"]
mod event_impl;
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
pub use event_impl::{Event, MessageEvent, MessageKind, Origin, RawLine, Source};
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
pub use papo_proto::{
    AccountName, Casemapping, ChannelName, Folded, InvalidIdentifier, Mask, MessageId, MetadataKey,
    Nick, Target, Typing,
};
pub use reconnect::Reconnect;
pub use requests::{ChannelInfo, ListEntry, ListFilter, WhoEntry, Whois};
pub use state::State;

/// Payloads carried by [`Event`].
pub mod event {
    pub use crate::event_impl::{
        AccountChanged, AwayChanged, BatchAbandonReason, CapsChanged, ChannelRenamed,
        ChannelSynced, Departed, Disconnected, HostChanged, Invited, Joined, Kicked,
        MetadataChanged, ModeChange, ModeChanged, MonitorOffline, MonitorOnline, Netjoin, Netsplit,
        NickChanged, Parted, PersistenceChanged, PersistenceSetting, Quit, Reacted, ReadMarker,
        RealnameChanged, Redacted, Registered, Source, StandardReply, StandardReplyKind,
        StateLimit, TagMsg, TopicChanged, TypingChanged, Warning,
    };
}

/// Strict Transport Security policy stores.
pub mod sts;
