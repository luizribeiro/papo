//! Tokio-based IRC client orchestration, transports, and ergonomic handles.
//!
//! TLS uses ring when both `tls-ring` and `tls-aws-lc` are enabled.
//! APIs gated by the `unstable` feature are outside the crate's semantic-versioning guarantees.
//! Applications can opt in while libraries should leave the feature disabled.
//! Browser builds (`wasm32-unknown-unknown`) connect only through `ws://` and `wss://`
//! [`Endpoint::websocket`] endpoints. The browser owns TLS policy, and IRC STS does not apply to
//! WebSocket connections, so TCP endpoint constructors, TLS setters, and STS stores are absent.
//!
//! # Frozen re-exports
//!
//! Identifier, state-model, authentication, capability, casemapping, typing, rate-limit, and STS
//! types re-exported by this crate are part of `papo`'s public compatibility contract. The
//! internal `papo-core` and `papo-proto` dependencies are pinned exactly so their implementations
//! cannot change independently of this facade.

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
pub mod fail;
pub mod format;
#[cfg(not(browser))]
mod io_compat;
pub mod isupport;
pub mod metadata;
mod mode;
pub mod numeric;
mod reconnect;
mod requests;
mod rt;
mod state;
pub mod time;
mod transport;
mod webirc;

#[doc(hidden)]
#[path = "testing.rs"]
pub mod __testing;

#[cfg(feature = "bot")]
pub mod bot;
pub mod cap;
pub mod tags;

/// The version of the `papo` crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use action::RequestFuture;
pub use address::{Endpoint, EndpointKind, IntoEndpoint, InvalidInput};
pub use builder::Builder;
pub use client::{Client, Fallback, JoinChannel, SendMessage, Sent};
pub use driver::{Driver, Events};
pub use error::{
    ConnectError, ConnectErrorKind, DisconnectCause, Disconnected, Error, ServerError,
};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use event_impl::PersistenceSetting;
pub use event_impl::{Ctcp, Event, Message, MessageKind, ModeKind, Origin, Source, Tags};
pub use ext::accounts::{AccountCreation, NewAccount};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::bouncer::Bouncer;
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
pub use ext::persistence::PersistenceStatus;
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::webpush::{PushKeys, WebPush};
pub use isupport::Isupport;
pub use metadata::MetadataValue;
pub use mode::SetModes;
#[doc(inline)]
pub use papo_core::Refusal;
#[doc(inline)]
pub use papo_core::Secret;
#[doc(inline)]
pub use papo_core::{Account, Away, Capabilities, Channel, Feature, Member, User};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use papo_core::{BouncerNetwork, NetworkConfig, NetworkId, NetworkState};
#[doc(inline)]
pub use papo_core::{InvalidRateLimit, RateLimit, Sasl};
#[doc(inline)]
pub use papo_core::{Unsupported, UnsupportedReason};
pub use papo_proto::Address as Target;
#[doc(inline)]
pub use papo_proto::{
    AccountName, Casemapping, ChannelName, InvalidIdentifier, Line, Mask, MessageId, MetadataKey,
    Nick, Typing,
};
pub use reconnect::Reconnect;
pub use requests::{ListEntry, ListFilter, WhoEntry, Whois, WhoisChannel};
pub use state::State;
pub use webirc::WebIrc;

/// Payloads carried by [`Event`].
pub mod event {
    pub use crate::event_impl::{
        AccountChanged, AwayChanged, BatchAbandonReason, CapsChanged, CasemappingChanged,
        ChannelRenamed, ChannelSynced, Connected, Departed, Disconnected, FeaturesChanged,
        HostChanged, Invited, Joined, Kicked, MetadataChanged, ModeChange, ModeChanged,
        MonitorOffline, MonitorOnline, Netjoin, Netsplit, NickChanged, Parted, Quit, Reacted,
        ReadMarker, RealnameChanged, Redacted, Source, SplitServers, StandardReply,
        StandardReplyKind, StateLimit, TagMsg, TopicChanged, TypingChanged, Visibility, Warning,
    };
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub use crate::event_impl::{BouncerNetworkChanged, PersistenceChanged, PersistenceSetting};
}

/// Strict Transport Security policy stores.
pub mod sts;
