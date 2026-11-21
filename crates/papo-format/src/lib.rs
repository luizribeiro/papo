//! Build, parse, and strip IRC text formatting. This crate has no stability promise.

mod formatting;

pub use formatting::{
    Color, FormatBuilder, Formatted, Span, Style, display_text, strip_formatting,
};
