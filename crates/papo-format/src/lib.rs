//! Build, parse, and strip IRC text formatting. This crate has no stability promise.

#![deny(missing_debug_implementations)]

mod formatting;

pub use formatting::{
    Color, FormatBuilder, Formatted, Span, Style, display_text, strip_formatting,
};
