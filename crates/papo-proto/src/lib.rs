//! IRC line parsing, formatting, casemapping, and typed protocol data with no I/O. Most users want
//! the `papo` crate; use `papo-proto` directly for low-level IRC wire processing.
//!
//! The IRC specifications call each line a message; this crate uses "line" so
//! "message" can consistently mean a chat message in the wider API.

#![cfg_attr(docsrs, feature(doc_cfg))]

mod casemapping;
mod client_tags;
mod command;
mod ctcp;
mod formatting;
mod id;
mod isupport;
mod line;
mod modes;
mod names;
mod numeric;
mod secret;
mod server_name;
mod source;
mod text;
pub mod time;
mod views;
mod whox;

pub use casemapping::Casemapping;
pub use client_tags::{Reaction, Typing};
pub use command::Command;
pub use ctcp::Ctcp;
pub use formatting::{Color, FormatBuilder, Formatted, Span, Style, strip_formatting};
pub use id::{
    AccountName, Address, ChannelName, InvalidIdentifier, Mask, MaskSource, MessageId, MetadataKey,
    Nick, Origin,
};
pub use isupport::{
    Advertised, ChanModes, ClientTagDeny, Extban, Isupport, Limit, LimitValue, Prefix,
};
pub use line::{
    BuildError, Line, LineRef, MAX_CLIENT_TAG_DATA_LEN, MAX_LINE_LEN, MAX_TAG_SECTION_LEN, Param,
    Params, ParseError, Tags, WireLine, escape_tag_value,
};

pub use modes::{ModeAction, ModeChange, ModeChanges, ModeKind};
pub use names::{NAME_ALIASES, NameKind, NamesEntry, canonical, parse_353};
pub use numeric::Numeric;
pub use secret::Secret;
pub use server_name::ServerName;
pub use source::Source;
pub use text::{Segment, split_text};
pub use views::{Account, Away, Chghost, Join, Setname, StandardReply, StandardReplyKind};
pub use whox::{WhoxFlags, WhoxPresence, WhoxReply};
