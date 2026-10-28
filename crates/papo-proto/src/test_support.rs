//! Inspection helpers for out-of-workspace fuzz targets.

use std::ops::Range;

use crate::WireLine;

/// Returns the byte ranges that must be redacted from formatted wire lines.
#[must_use]
pub fn wire_secret_spans(line: &WireLine) -> &[Range<usize>] {
    line.secret_spans()
}

/// Returns the encoded length before the terminating CRLF.
#[must_use]
pub fn wire_visible_len(line: &WireLine) -> usize {
    line.visible_len()
}
