//! IRC timestamp formatting helpers.

/// Formats a UTC timestamp as RFC 3339 with millisecond precision.
///
/// Returns `None` when the timestamp's year is outside `0000..=9999`.
pub use papo_proto::time::format;

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn formats_rfc3339_at_millisecond_precision() {
        let time = UNIX_EPOCH + Duration::new(1_770_091_506, 789_999_999);
        assert_eq!(
            super::format(time).as_deref(),
            Some("2026-02-03T04:05:06.789Z")
        );
    }
}
