//! IRC message parsing, formatting, casemapping, and typed protocol data.
//! This crate performs no I/O and has minimal dependencies.

mod command;
mod message;
mod numeric;
mod source;

pub use command::Command;
pub use message::{
    BuildError, MAX_CLIENT_TAG_DATA_LEN, MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message, MessageRef,
    Params, ParseError, Tags,
};
pub use numeric::Numeric;
pub use source::Source;
