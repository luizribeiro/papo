//! Parsing and formatting for `IRCv3` server-time timestamps.

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A millisecond-precision timestamp from an IRC server's clock.
///
/// This is intentionally distinct from [`SystemTime`], which applications use for this device's
/// clock. It can be converted to `SystemTime` for display or arithmetic, but the two clock domains
/// cannot accidentally be mixed in APIs that require a server timestamp.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ServerTime(i64);

/// A timestamp that is not a valid fixed-width `IRCv3` server-time value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidServerTime;

impl ServerTime {
    /// Restores a timestamp that originally came from this server.
    ///
    /// `value` must use the RFC 3339 form used by the `IRCv3` `time` tag. Precision finer than a
    /// millisecond is truncated because [`ServerTime`] has millisecond precision.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidServerTime`] when `value` is malformed or outside years `0000..=9999`.
    pub fn from_stored(value: &str) -> Result<Self, InvalidServerTime> {
        value.parse()
    }

    /// Restores milliseconds since the Unix epoch that originally came from this server.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidServerTime`] when the value is outside years `0000..=9999`.
    pub fn from_stored_millis(millis: i64) -> Result<Self, InvalidServerTime> {
        format_millis(millis)
            .is_some()
            .then_some(Self(millis))
            .ok_or(InvalidServerTime)
    }

    /// Returns milliseconds since the Unix epoch.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Converts this server-clock value for display or explicit arithmetic.
    #[must_use]
    pub fn to_system_time(self) -> SystemTime {
        if self.0 >= 0 {
            UNIX_EPOCH + Duration::from_millis(self.0.unsigned_abs())
        } else {
            UNIX_EPOCH - Duration::from_millis(self.0.unsigned_abs())
        }
    }
}

impl fmt::Display for ServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&format_millis(self.0).expect("ServerTime invariant"))
    }
}

impl FromStr for ServerTime {
    type Err = InvalidServerTime;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_millis(value).map(Self).ok_or(InvalidServerTime)
    }
}

impl fmt::Display for InvalidServerTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid IRCv3 server timestamp")
    }
}

impl std::error::Error for InvalidServerTime {}

/// Parses a fixed-width UTC timestamp with up to nanosecond precision.
#[must_use]
pub fn parse(value: &str) -> Option<ServerTime> {
    parse_millis(value).map(ServerTime)
}

fn parse_millis(value: &str) -> Option<i64> {
    let (seconds, nanos) = parse_parts(value)?;
    seconds
        .checked_mul(1_000)?
        .checked_add(i64::from(nanos / 1_000_000))
}

fn parse_parts(value: &str) -> Option<(i64, u32)> {
    let bytes = value.as_bytes();
    if !value.is_ascii()
        || !(bytes.len() == 20 || (22..=30).contains(&bytes.len()))
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[bytes.len() - 1] != b'Z'
    {
        return None;
    }
    let year = decimal(&bytes[0..4])?;
    let month = decimal(&bytes[5..7])?;
    let day = decimal(&bytes[8..10])?;
    let hour = decimal(&bytes[11..13])?;
    let minute = decimal(&bytes[14..16])?;
    let second = decimal(&bytes[17..19])?;
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        2 => 28 + i64::from(leap_year),
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if !(1..=month_days).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let fraction = if bytes.len() == 20 {
        0
    } else {
        if bytes[19] != b'.' {
            return None;
        }
        let digits = &bytes[20..bytes.len() - 1];
        let padding = u32::try_from(9 - digits.len()).ok()?;
        u32::try_from(decimal(digits)?).ok()? * 10_u32.pow(padding)
    };
    let nanos = if second == 60 { 999_999_999 } else { fraction };
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let days = era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + day_of_year
        - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second.min(59);
    Some((seconds, nanos))
}

/// Formats a server timestamp as RFC 3339 at millisecond precision.
#[must_use]
pub fn format(time: ServerTime) -> String {
    time.to_string()
}

fn format_millis(millis: i64) -> Option<String> {
    let seconds = millis.div_euclid(1_000);
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100 + year_of_era / 400);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let second_of_day = seconds.rem_euclid(86_400);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60,
        millis.rem_euclid(1_000)
    ))
}

fn decimal(bytes: &[u8]) -> Option<i64> {
    bytes.iter().try_fold(0_i64, |value, byte| {
        byte.is_ascii_digit()
            .then_some(value * 10 + i64::from(byte - b'0'))
    })
}

#[cfg(test)]
mod tests {
    use super::ServerTime;

    #[test]
    fn parses_and_formats_millisecond_precision() {
        let time = ServerTime::from_stored_millis(1_770_091_506_789).unwrap();
        assert_eq!(super::parse("2026-02-03T04:05:06.789Z"), Some(time));
        assert_eq!(time.to_string(), "2026-02-03T04:05:06.789Z");
    }
}
