//! IRC message parsing, formatting, casemapping, and typed protocol data.
//! This crate performs no I/O and has minimal dependencies.

mod message;

pub use message::{MAX_MESSAGE_LEN, MAX_TAG_SECTION_LEN, MessageRef, Params, ParseError, Tags};
