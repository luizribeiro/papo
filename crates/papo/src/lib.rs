//! Tokio-based IRC client orchestration, transports, and ergonomic handles.
//!
//! TLS uses ring when both `tls-ring` and `tls-aws-lc` are enabled.
//! APIs gated by the `unstable` feature are outside the crate's semantic-versioning guarantees.
//! Applications can opt in while libraries should leave the feature disabled.

#![cfg_attr(docsrs, feature(doc_cfg))]

mod action;
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
pub mod metadata;
mod mode;
mod reconnect;
mod requests;
mod rt;
mod state;
mod transport;

#[cfg(feature = "bot")]
pub mod bot;
pub mod cap;

pub use action::RequestFuture;
pub use address::{Endpoint, EndpointKind, IntoEndpoint, InvalidInput};
pub use builder::Builder;
pub use client::{Client, JoinChannel, SendMessage, Sent};
pub use driver::{Driver, Events};
pub use entropy::OsEntropy;
pub use error::{
    ConnectError, ConnectErrorKind, DisconnectCause, Disconnected, Error, Refusal,
    RegistrationError, ServerError,
};
pub use event_impl::{Event, Message, MessageKind, Origin, RawLine, Source, Tags};
pub use ext::accounts::{AccountRegistration, NewAccount};
pub use ext::history::{
    History, HistoryItem, HistoryPage, HistoryReference, HistoryRequest, HistoryTarget,
};
pub use ext::metadata::Metadata;
pub use ext::monitor::Monitor;
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::persistence::Persistence;
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::webpush::{PushKeys, WebPush};
pub use mode::SetModes;
pub use papo_core::Secret;
pub use papo_core::WebIrc;
pub use papo_core::{Account, Capabilities, Channel, Feature, Member, User};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use papo_core::{BouncerNetwork, NetworkConfig, NetworkId, NetworkState};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use papo_core::{PersistenceSetting, PersistenceStatus};
pub use papo_core::{RateLimit, Sasl};
pub use papo_core::{Unsupported, UnsupportedReason};
pub use papo_proto::{
    AccountName, Casemapping, ChannelName, Folded, InvalidIdentifier, Mask, MessageId, MetadataKey,
    ModeKind, Nick, Target, Typing,
};
pub use reconnect::Reconnect;
pub use requests::{ListEntry, ListFilter, WhoEntry, Whois, WhoisChannel};
pub use state::State;

/// Payloads carried by [`Event`].
pub mod event {
    pub use crate::event_impl::{
        AccountChanged, AwayChanged, BatchAbandonReason, CapsChanged, ChannelRenamed,
        ChannelSynced, Departed, Disconnected, FeaturesChanged, HostChanged, Invited, Joined,
        Kicked, MetadataChanged, ModeChange, ModeChanged, MonitorOffline, MonitorOnline, Netjoin,
        Netsplit, NickChanged, Parted, Quit, Reacted, ReadMarker, RealnameChanged, Redacted,
        Registered, Source, StandardReply, StandardReplyKind, StateLimit, TagMsg, TopicChanged,
        TypingChanged, Visibility, Warning,
    };
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub use crate::event_impl::{BouncerNetworkChanged, PersistenceChanged, PersistenceSetting};
}

/// Strict Transport Security policy stores.
pub mod sts;
