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
//!
//! # Public type paths
//!
//! Handles and types useful outside an [`Event`] are at the crate root. Types used only as event
//! payloads are in [`event`]. Feature modules contain their feature-specific helpers. Each public
//! type has one public path; for example, [`Disconnected`] is at the root while [`event::Motd`]
//! and [`event::CasemappingChanged`] are event payloads.

#![cfg_attr(docsrs, feature(doc_cfg))]

macro_rules! variant_accessors {
    ($type:ident;) => {};
    ($type:ident; (boxed $variant:ident, $method:ident, $payload:ty), $($rest:tt)*) => {
        impl $type {
            #[doc = concat!("Returns the payload when this is [`", stringify!($type), "::", stringify!($variant), "`].")]
            #[must_use]
            pub fn $method(&self) -> Option<&$payload> {
                if let Self::$variant(payload) = self {
                    Some(payload)
                } else {
                    None
                }
            }
        }
        variant_accessors! { $type; $($rest)* }
    };
    ($type:ident; (field $variant:ident, $field:ident, $method:ident, $payload:ty), $($rest:tt)*) => {
        impl $type {
            #[doc = concat!("Returns the payload when this is [`", stringify!($type), "::", stringify!($variant), "`].")]
            #[must_use]
            pub const fn $method(&self) -> Option<&$payload> {
                if let Self::$variant { $field, .. } = self {
                    Some($field)
                } else {
                    None
                }
            }
        }
        variant_accessors! { $type; $($rest)* }
    };
    ($type:ident; ($variant:ident, $method:ident, $payload:ty), $($rest:tt)*) => {
        impl $type {
            #[doc = concat!("Returns the payload when this is [`", stringify!($type), "::", stringify!($variant), "`].")]
            #[must_use]
            pub const fn $method(&self) -> Option<&$payload> {
                if let Self::$variant(payload) = self {
                    Some(payload)
                } else {
                    None
                }
            }
        }
        variant_accessors! { $type; $($rest)* }
    };
}

pub(crate) fn opaque_debug(
    name: &str,
    formatter: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    formatter.debug_struct(name).finish_non_exhaustive()
}

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
#[cfg(not(browser))]
mod io_compat;
#[path = "isupport.rs"]
mod isupport_impl;
mod message_ref;
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

pub mod cap;
pub mod tags;

/// Read-only views of server features advertised through `RPL_ISUPPORT`.
pub mod isupport {
    #[doc(inline)]
    pub use papo_proto::{Advertised, ChanModes, ClientTagDeny, Extban, Limit, LimitValue, Prefix};
}

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
pub use event_impl::{Authorship, Event, Message, MessageKind, Origin, Source, Tags};
pub use ext::accounts::{AccountCreation, NewAccount};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::bouncer::Bouncer;
pub use ext::history::{
    ConversationInput, History, HistoryItem, HistoryPage, HistoryReference, HistoryRequest,
    RecentConversation,
};
pub use ext::metadata::Metadata;
pub use ext::monitor::Monitor;
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::persistence::Persistence;
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use ext::webpush::{PushKeys, WebPush};
pub use isupport_impl::Isupport;
pub use message_ref::MessageRef;
pub use mode::{BanTarget, Extban, KickbanOutcome, ModesApplied, SetModes};
#[doc(inline)]
pub use papo_core::Secret;
#[doc(inline)]
pub use papo_core::{
    Account, Away, Cap, Capabilities, Channel, Conversation, Feature, Member, ModeAction, ModeArg,
    ModeChange, ModeValue, Rank, Typer, User,
};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use papo_core::{BouncerNetwork, NetworkConfig, NetworkId, NetworkState};
#[doc(inline)]
pub use papo_core::{CtcpReplies, InvalidRateLimit, RateLimit, Sasl, WhoOnJoin};
#[doc(inline)]
pub use papo_core::{ListMode, ModeListEntry, Refusal, StateLimit};
pub use papo_core::{MetadataValue, Visibility};
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
pub use papo_core::{PersistenceSetting, PersistenceStatus};
#[doc(inline)]
pub use papo_core::{Unsupported, UnsupportedReason};
#[doc(inline)]
pub use papo_proto::{
    AccountName, Casemapping, ChannelName, Color, FormatBuilder, InvalidIdentifier, Line, Mask,
    MessageId, MetadataKey, Nick, ServerName, Span, Style, Typing, strip_formatting,
};
pub use reconnect::Reconnect;
pub use requests::{ListEntry, ListFilter, WhoEntry, Whois, WhoisChannel};
pub use state::State;
pub use webirc::WebIrc;

/// A user or channel acted on by an IRC event.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Subject {
    /// A channel acted on by the event.
    Channel(ChannelName),
    /// A user acted on by the event.
    User(Nick),
}

/// Payloads carried by [`Event`].
pub mod event {
    #[cfg(feature = "unstable")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
    pub use crate::event_impl::BouncerNetworkChanged;
    pub use crate::event_impl::{
        AccountChanged, AwayChanged, BatchAbandonReason, CapsChanged, CasemappingChanged,
        ChannelRenamed, ChannelSynced, Connected, Ctcp, FeaturesChanged, HostChanged, Invited,
        Joined, Kicked, MembersRefreshed, MetadataChanged, ModeChanged, MonitorOffline,
        MonitorOnline, Motd, Netjoin, Netsplit, NickChanged, Parted, Quit, Reacted,
        ReadMarkerChanged, RealnameChanged, Redacted, SplitServers, StandardReply,
        StandardReplyKind, TagMsg, TopicChanged, TypingChanged, TypingReason, Warning,
    };
    pub use crate::event_impl::{ConnectPhase, Connecting};
}

pub(crate) use event_impl::ConnectPhase;

/// Strict Transport Security policy stores.
pub mod sts;
