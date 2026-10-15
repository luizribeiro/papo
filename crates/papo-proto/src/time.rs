//! Parsing and formatting for `IRCv3` server-time timestamps.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Parses a fixed-width UTC timestamp with up to nanosecond precision.
#[must_use]
pub fn parse(value: &str) -> Option<SystemTime> {
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
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(u64::try_from(seconds).ok()?, nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(seconds.unsigned_abs()))?
            .checked_add(Duration::from_nanos(u64::from(nanos)))
    }
}

/// Formats a UTC timestamp at millisecond precision.
#[must_use]
pub fn format(time: SystemTime) -> Option<String> {
    let millis = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_millis()),
        Err(error) => i128::try_from(error.duration().as_millis()).map(|value| -value),
    }
    .ok()?;
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
    year += i128::from(month <= 2);
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
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn parses_and_formats_millisecond_precision() {
        let time = UNIX_EPOCH + Duration::new(1_770_091_506, 789_000_000);
        assert_eq!(super::parse("2026-02-03T04:05:06.789Z"), Some(time));
        assert_eq!(
            super::format(time).as_deref(),
            Some("2026-02-03T04:05:06.789Z")
        );
    }
}
