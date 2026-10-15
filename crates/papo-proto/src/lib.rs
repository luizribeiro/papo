//! IRC message parsing, formatting, casemapping, and typed protocol data.
//! This crate performs no I/O and has minimal dependencies.

mod message;

pub use message::{
    BuildError, MAX_CLIENT_TAG_DATA_LEN, MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, Message, MessageRef,
    Params, ParseError, Tags,
};
