//! IRC message parsing, formatting, casemapping, and typed protocol data.
//! This crate performs no I/O and has minimal dependencies.

mod message;

pub use message::{MessageRef, Params, ParseError, Tags};
