//! IRC line parsing, formatting, casemapping, and typed protocol data.
//!
//! The IRC specifications call each line a message; this crate uses "line" so
//! "message" can consistently mean a chat message in the wider API.
//! This crate performs no I/O and has minimal dependencies.

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
    AccountName, ChannelName, InvalidIdentifier, Mask, MessageId, MetadataKey, Nick, Target,
};
pub use isupport::{
    Advertised, ChanModes, ClientTagDeny, Extban, Isupport, Limit, LimitValue, Prefix,
};
pub use line::{
    BuildError, Line, LineRef, MAX_CLIENT_TAG_DATA_LEN, MAX_LINE_LEN, MAX_TAG_SECTION_LEN, Params,
    ParseError, Tags, escape_tag_value,
};

pub use modes::{ModeChange, ModeChanges, ModeKind};
pub use names::{NAME_ALIASES, NameKind, NamesEntry, canonical, parse_353};
pub use numeric::Numeric;
pub use source::Source;
pub use text::{Segment, split_text};
pub use views::{Account, Away, Chghost, Join, Setname, StandardReply, StandardReplyKind};
pub use whox::{WhoxFlags, WhoxPresence, WhoxReply};

/// An IRC wire address chosen by the caller.
pub type Address = Target;
