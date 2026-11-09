//! IRC timestamp formatting helpers.

use std::time::SystemTime;

/// Returns the current wall-clock time from this device's clock.
///
/// This is available on every supported target, including browsers where
/// [`SystemTime::now`] is not implemented.
#[must_use]
pub fn now() -> SystemTime {
    crate::rt::now_wall()
}

/// Formats a server timestamp as RFC 3339 with millisecond precision.
pub use papo_proto::time::format;

#[cfg(all(test, not(browser)))]
mod tests {
    use std::time::SystemTime;

    #[test]
    fn reads_the_current_wall_clock() {
        let before = SystemTime::now();
        let actual = super::now();
        let after = SystemTime::now();
        assert!(actual >= before);
        assert!(actual <= after);
    }

    #[test]
    fn formats_rfc3339_at_millisecond_precision() {
        let time = crate::ServerTime::from_stored_millis(1_770_091_506_789).unwrap();
        assert_eq!(super::format(time), "2026-02-03T04:05:06.789Z");
    }
}
