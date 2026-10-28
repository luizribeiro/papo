//! An ergonomic async IRC client for applications and bots, with `IRCv3` support and Tokio-based
//! I/O on native and WASI targets.
//!
//! # Quick start
//!
//! ```no_run
//! use papo::{Client, Event};
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let (client, mut events) = Client::builder("ircs://irc.example.net", "papo-example")
//!         .autojoin(["#papo"])
//!         .connect()
//!         .await?;
//!
//!     while let Some(event) = events.next().await {
//!         if let Event::Message(message) = event
//!             && message.text == "!ping"
//!             && let Err(error) = client.reply(&message, "pong").await
//!         {
//!             eprintln!("could not reply: {error}");
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! # Examples
//!
//! Complete applications in the repository include:
//!
//! - [Ping bot](https://github.com/luizribeiro/papo/tree/main/examples/ping-bot)
//! - [Reaction bot](https://github.com/luizribeiro/papo/tree/main/examples/react-bot)
//! - [Admin bot](https://github.com/luizribeiro/papo/tree/main/examples/admin-bot)
//! - [Lurker](https://github.com/luizribeiro/papo/tree/main/examples/lurker)
//! - [Catchup](https://github.com/luizribeiro/papo/tree/main/examples/catchup)
//! - [Create account](https://github.com/luizribeiro/papo/tree/main/examples/create-account)
//! - [Bouncer networks](https://github.com/luizribeiro/papo/tree/main/examples/bouncer-networks)
//! - [Persist STS policies](https://github.com/luizribeiro/papo/tree/main/examples/sts-store)
//! - [Graceful shutdown](https://github.com/luizribeiro/papo/tree/main/examples/graceful-shutdown)
//! - [Browser SharedWorker](https://github.com/luizribeiro/papo/tree/main/examples/browser-worker)
//!
//! # Testing applications
//!
//! Use [`papo-testing`](https://docs.rs/papo-testing) for deterministic tests of a real [`Client`]
//! against an in-memory IRC server. Its crate documentation also covers end-to-end tests against
//! a local Ergo, including native and browser WebSocket endpoints.
//!
//! # Cargo features
//!
//! | Feature | What it adds | Targets |
//! |---|---|---|
//! | `default` | The `tls-ring` feature | Native and WASI |
//! | `precis` | Unicode nickname and account comparison using PRECIS | All targets |
//! | `websocket` | `ws://` and `wss://` transports | Native and WASI; browsers provide WebSocket support without this feature |
//! | `tls-ring` | TLS through rustls with the ring provider | Native and WASI |
//! | `tls-aws-lc` | TLS through rustls with the AWS-LC provider | Native and WASI |
//! | `unstable` | Experimental extension APIs outside semantic-versioning guarantees | All targets |
//!
//! If both TLS provider features are enabled, ring is used.
//!
//! APIs gated by the `unstable` feature are outside the crate's semantic-versioning guarantees.
//! Applications can opt in while libraries should leave the feature disabled.
//!
//! # `IRCv3` support
//!
//! <!-- caps:start -->
//! | Capability | Aliases | Requested | Features | Code |
//! |---|---|---|---|---|
//! | `cap-notify` | — | default | capability notifications | `caps` |
//! | `message-tags` | — | default | client-only message tags, replies, reactions, typing notifications | `features`, `replies` |
//! | `server-time` | — | default | server timestamps | `state::read_marker` |
//! | `batch` | — | default | message batches | `batch` |
//! | `draft/event-playback` | — | default | event playback | `batch` |
//! | `echo-message` | — | default | message echoes | `outgoing`, `echo` |
//! | `account-tag` | — | default | account tracking | `state::presence` |
//! | `extended-join` | — | default | extended join details | `state::membership` |
//! | `multi-prefix` | — | default | complete membership prefixes | `state::membership` |
//! | `userhost-in-names` | — | default | user and host fields in names replies | `state::membership` |
//! | `chghost` | — | default | host tracking | `state::presence` |
//! | `setname` | — | default | real-name changes | `state::presence` |
//! | `away-notify` | — | default | away tracking | `state::presence` |
//! | `account-notify` | — | default | account tracking | `state::presence` |
//! | `extended-monitor` | — | default | extended monitor replies | `monitor` |
//! | `invite-notify` | — | default | invite tracking | `replies` |
//! | `standard-replies` | — | default | structured replies | `refusal`, `replies` |
//! | `labeled-response` | — | default | labeled-response | `outgoing`, `labels` |
//! | `draft/multiline` | — | default | multiline messages | `outgoing`, `batch` |
//! | `draft/oper-tag` | — | default | operator tags | `replies` |
//! | `draft/metadata-3` | draft/metadata-2 | default | metadata | `metadata`, `state::metadata` |
//! | `metadata-notify` | — | fallback | — | `metadata`, `state::metadata` |
//! | `sasl` | — | automatic | SASL authentication | `sasl` |
//! | `sts` | — | never | — | `sts` |
//! | `ergo.chat/nope` | — | never | — | `caps` |
//! | `soju.im/account-required` | — | never | — | `caps` |
//! | `draft/account-registration` | account-registration | default | account creation | `account_registration` |
//! | `soju.im/bouncer-networks` | — | opt-in | bouncer networks | `bouncer` |
//! | `soju.im/bouncer-networks-notify` | — | opt-in | — | `bouncer` |
//! | `draft/chathistory` | chathistory | default | chathistory | `history`, `batch` |
//! | `draft/read-marker` | read-marker | default | read-marker | `history`, `state::read_marker` |
//! | `draft/persistence` | — | opt-in | persistence | `persistence`, `state::persistence` |
//! | `draft/channel-rename` | — | default | channel rename | `channel_rename`, `state::rename` |
//! | `draft/relaymsg` | — | opt-in | relaymsg | `relaymsg` |
//! | `draft/message-redaction` | — | default | draft/message-redaction | `outgoing`, `replies` |
//! | `draft/webpush` | soju.im/webpush | opt-in | webpush | `webpush` |
//! | `draft/pre-away` | — | default | registration-time away state | `state` |
//! | `no-implicit-names` | draft/no-implicit-names | default | suppression of implicit names replies | `state::membership` |
//! | `draft/whoami` | — | default | connection identity discovery | `state` |
//! | `draft/extended-isupport` | — | default | extended ISUPPORT discovery | `isupport` |
//! <!-- caps:end -->
//!
//! # Running and shutting down
//!
//! [`Builder::connect`] spawns the [`Driver`] and waits for the first connection. On native
//! targets it must run inside a Tokio runtime and panics otherwise. It is safe to wrap this future
//! in a timeout; [`Builder::connect_timeout`] also bounds each transport connection attempt.
//!
//! [`Builder::build`] returns the [`Client`], [`Events`], and lazy [`Driver`] separately. Poll or
//! spawn the driver before awaiting a client action: awaiting `client.join("#papo")` before
//! spawning the driver deadlocks because the action is waiting for that driver. [`Events::next`]
//! is cancel-safe, so it may be used as a `tokio::select!` branch.
//!
//! The connection remains alive while either a [`Client`] or its [`Events`] stream is alive.
//! Dropping only `Events` stops delivery while a `Client` keeps the connection alive; dropping
//! every `Client` still leaves it alive while `Events` is retained. Dropping the final handle asks
//! the driver to send `QUIT`, but `Drop` cannot await delivery or acknowledgement. Applications
//! should call [`Client::quit`] explicitly when they choose to shut down, then keep reading events
//! until the stream ends.
//!
//! Dropping a [`RequestFuture`] does not cancel work that has reached the driver. In particular,
//! multi-step actions continue through their remaining protocol commands. Dropping the driver
//! ends the event stream without a final [`Event::Disconnected`]. For an orderly process exit,
//! send [`Client::quit`] and keep reading events until the server closes the connection. The
//! [graceful-shutdown example](https://github.com/luizribeiro/papo/tree/main/examples/graceful-shutdown)
//! shows the complete lifecycle.
//!
//! # WASI
//!
//! `wasm32-wasip2` and `wasm32-wasip3` need `--cfg tokio_unstable`; this repository sets it for
//! WASI targets in `.cargo/config.toml`. Wasmtime needs `-S inherit-network -S
//! allow-ip-name-lookup` for outbound networking and name lookup, plus `-S p3` for wasip3.
//!
//! # Browser builds
//!
//! For a size-oriented `wasm32-unknown-unknown` release build, add this profile to the
//! application's `Cargo.toml`:
//!
//! ```toml
//! [profile.release]
//! codegen-units = 1
//! lto = true
//! opt-level = "z"
//! panic = "abort"
//! ```
//!
//! On papo's core browser probe, these settings reduced the output after `wasm-opt -Oz` from
//! 996,188 to 701,675 bytes, a 29.6% reduction. Run
//! `wasm-opt -Oz input.wasm -o output.wasm` on the linked module before deployment.
//!
//! The high-level client runs in dedicated workers, shared workers, and Node.js using the
//! browser's WebSocket implementation. Pass a `ws://` or `wss://` URL directly to
//! [`Client::builder`]. TCP [`Endpoint`] constructors remain available for portable
//! configuration, but connecting through one returns an unavailable-transport error. The browser
//! owns TLS policy for `wss://`, and IRC STS does not apply to WebSocket connections, so custom
//! TLS setters and STS stores are absent from browser builds.
//!
//! # Frozen re-exports
//!
//! `papo` is the public compatibility surface. Its re-exported identifier, state-model,
//! authentication, capability, casemapping, typing, rate-limit, and STS types are part of that
//! contract. The published `papo-core` and `papo-proto` dependencies are internal implementation
//! crates with no API stability guarantees and are not intended for direct use.
//!
//! Re-exported output values cannot be constructed or exhaustively destructured outside `papo`:
//!
//! ```compile_fail
//! let change = papo::ModeChange {
//!     mode: 'o',
//!     action: papo::ModeAction::Add,
//!     arg: None,
//! };
//! ```
//!
//! ```compile_fail
//! # fn inspect(change: papo::ModeChange) {
//! let papo::ModeChange { mode, action, arg } = change;
//! # let _ = (mode, action, arg);
//! # }
//! ```
//!
//! # Public type paths
//!
//! Handles and types useful outside an [`Event`] are at the crate root. Types used only as event
//! payloads are in [`event`]. Feature modules contain their feature-specific helpers. Each public
//! type has one public path; for example, [`Disconnected`] is at the root while [`event::Motd`]
//! and [`event::CasemappingChanged`] are event payloads.
//!
//! # Argument conventions
//!
//! Text retained by `papo` takes `impl Into<String>`, while text inspected only for the duration
//! of a call is borrowed as `&str` or `impl AsRef<str>`. Retained lists similarly take
//! `IntoIterator` with items implementing `Into<String>`.
//!
//! # Client-bound values
//!
//! Typed names, direct handles, message references, and history cursors retain the [`Client`]
//! that produced them. Passing one to another client returns
//! [`InvalidInput::ForeignReference`]. Same-named channels from two clients therefore compare
//! unequal, although each compares equal to the same plain string. Converting a typed name into
//! a string explicitly discards this protection.
//!
//! The sealed [`Target`] trait accepts channel text, typed channels, guarded direct handles, and
//! return addresses. Nickname text and typed nicknames are refused as message targets; use
//! [`Client::direct`] to capture a guarded recipient or [`Client::send_raw`] as the explicit
//! escape hatch for an unguarded protocol line.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(all(test, browser))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

macro_rules! variant_accessors {
    ($type:ident;) => {};
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
mod direct;
mod direct_runtime;
mod direct_to;
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
mod name_rules;
pub mod numeric;
mod reconnect;
mod requests;
mod rt;
mod state;
mod target;
pub mod time;
mod transport;
mod webirc;

#[doc(hidden)]
#[path = "testing.rs"]
pub mod __testing;

mod cap;
pub mod tags;

/// Read-only views of server features advertised through `RPL_ISUPPORT`.
pub mod isupport {
    #[doc(inline)]
    pub use papo_proto::{Advertised, ChanModes, ClientTagDeny, ExtbanSyntax, Limit, LimitValue};
}

/// The version of the `papo` crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use action::RequestFuture;
pub use address::{Endpoint, EndpointKind, IntoEndpoint, InvalidInput};
pub use builder::{Builder, StreamSecurity};
pub use client::{Client, Fallback, JoinChannel, SendMessage, Sent, SentOutcome};
pub use direct::DirectRef;
pub use driver::{Driver, Events};
pub use error::{
    ConnectError, ConnectErrorKind, DisconnectCause, Disconnected, Error, ServerError,
};
pub use event_impl::{
    Authorship, Event, Message, MessageKind, Oper, Source, Tags, TagsIter, UserSource,
};
pub use ext::accounts::{AccountCreation, NewAccount};
pub use ext::history::{
    History, HistoryPage, HistoryReason, HistoryReference, HistoryRequest, RecentConversation,
};
pub use ext::metadata::Metadata;
pub use ext::monitor::Monitor;
#[cfg(feature = "unstable")]
pub use ext::{
    bouncer::{AddNetwork, Bouncer, ChangeNetwork},
    persistence::Persistence,
    webpush::{PushKeys, WebPush},
};
pub use isupport_impl::Isupport;
pub use message_ref::{MessageRef, NoticeRef};
pub use mode::{BanTarget, Extban, KickbanOutcome, ModesApplied, SetModes};
pub use name_rules::{NameCheck, NameRules};
#[doc(inline)]
pub use papo_core::InvalidUrl;
#[doc(inline)]
pub use papo_core::Secret;
#[doc(hidden)]
pub use papo_core::cap::__is_known as __is_known_cap;
#[doc(inline)]
pub use papo_core::{
    Account, Away, Cap, Capabilities, Channel, Conversation, Feature, Member, MetadataView,
    ModeAction, ModeArg, ModeChange, ModeValue, Rank, Typer, Typers, User,
};
#[cfg(feature = "unstable")]
pub use papo_core::{
    BouncerNetwork, NetworkId, NetworkState, PersistenceSetting, PersistenceStatus,
};
#[doc(inline)]
pub use papo_core::{CtcpReplies, InvalidRateLimit, RateLimit, Sasl, WhoOnJoin};
#[doc(inline)]
pub use papo_core::{ListMode, ModeListEntry, Refusal, StateLimit};
pub use papo_core::{MetadataValue, Visibility};
#[doc(inline)]
pub use papo_core::{Unsupported, UnsupportedReason};
#[doc(inline)]
pub use papo_proto::Prefix;
#[doc(inline)]
pub use papo_proto::{
    AccountName, AsAccount, AsChannel, BuildError, Casemapping, ChannelName, Color, FormatBuilder,
    InvalidIdentifier, Line, Mask, MessageId, MetadataKey, Nick, Origin, ServerName, Span, Style,
    Typing, UnknownCasemapping, strip_formatting,
};
pub use reconnect::Reconnect;
pub use requests::{ListEntry, ListRequest, WhoEntry, Whois, WhoisChannel};
pub use state::{ReadMarker, State};
pub use target::{AsConversation, AsMask, AsNick, Target};
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

impl Subject {
    /// Returns the user nickname when this subject is a user.
    #[must_use]
    pub const fn nick(&self) -> Option<&Nick> {
        match self {
            Self::User(nick) => Some(nick),
            Self::Channel(_) => None,
        }
    }

    /// Returns the channel name when this subject is a channel.
    #[must_use]
    pub const fn channel(&self) -> Option<&ChannelName> {
        match self {
            Self::Channel(channel) => Some(channel),
            Self::User(_) => None,
        }
    }
}

impl PartialEq<str> for Subject {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::Channel(channel) => channel == other,
            Self::User(nick) => nick == other,
        }
    }
}

impl PartialEq<&str> for Subject {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

/// Payloads carried by [`Event`].
pub mod event {
    #[cfg(feature = "unstable")]
    pub use crate::event_impl::BouncerNetworkChanged;
    pub use crate::event_impl::{
        AccountChanged, Audience, AutoJoinFailed, AwayChanged, BatchAbandonReason, Broadcast,
        BroadcastCtcp, BroadcastTagMsg, CapsChanged, CasemappingChanged, ChannelRenamed,
        ChannelSynced, Connected, CtcpQuery, CtcpReply, FeaturesChanged, HostChanged, Invited,
        Joined, Kicked, Lagged, MembersRefreshed, MessageGap, MetadataChanged, ModeChanged,
        MonitorOffline, MonitorOnline, Motd, NickChanged, Notice, Parted, Quit, Reacted,
        ReactionAction, ReadMarkerChanged, RealnameChanged, Redacted, ServerNotice,
        ServerNoticeKind, SplitServers, StandardReply, StandardReplyKind, TagMsg, TopicChanged,
        TypingChanged, TypingReason, Unparsed, Warning,
    };
    pub use crate::event_impl::{ConnectPhase, Connecting};
}

pub(crate) use event_impl::ConnectPhase;

/// Strict Transport Security policy stores.
pub mod sts;
