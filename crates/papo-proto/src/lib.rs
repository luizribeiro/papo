//! IRC message parsing, formatting, casemapping, and typed protocol data.
//! This crate performs no I/O and has minimal dependencies.

mod casemapping;
mod client_tags;
mod command;
mod ctcp;
mod formatting;
mod id;
mod isupport;
mod message;
mod modes;
mod names;
mod numeric;
mod source;
mod text;
pub mod time;
mod views;
mod whox;

pub use casemapping::{CaseKey, Casemapping};
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
pub use message::{
    BuildError, MAX_CLIENT_TAG_DATA_LEN, MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message, MessageRef,
    Params, ParseError, Tags, escape_tag_value,
};
pub use modes::{ModeChange, ModeChanges, ModeKind};
pub use names::{NAME_ALIASES, NameKind, NamesEntry, canonical, parse_353};
pub use numeric::Numeric;
pub use source::Source;
pub use text::{Segment, split_text};
pub use views::{Account, Away, Chghost, Join, Setname, StandardReply, StandardReplyKind};
pub use whox::{WhoxFlags, WhoxPresence, WhoxReply};
