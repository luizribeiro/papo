//! Device wall-clock time.
//!
//! Server times are [`crate::ServerTime`]; format one with its `Display` impl.

use std::time::SystemTime;

/// Returns the current wall-clock time from this device's clock.
///
/// This is available on every supported target, including browsers where
/// [`SystemTime::now`] is not implemented.
#[must_use]
pub fn now() -> SystemTime {
    crate::rt::now_wall()
}

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
}
