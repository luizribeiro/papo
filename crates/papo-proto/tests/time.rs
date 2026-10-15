//! Boundary coverage for `IRCv3` server-time timestamps.

use std::time::{Duration, UNIX_EPOCH};

use papo_proto::time;
use proptest::prelude::*;

#[test]
fn parses_fixed_utc_server_times() {
    for (input, seconds, nanos) in [
        ("2026-02-03T04:05:06.789Z", 1_770_091_506, 789_000_000),
        ("2026-02-03T04:05:06Z", 1_770_091_506, 0),
        ("2026-02-03T04:05:06.123456789Z", 1_770_091_506, 123_456_789),
        ("1970-01-01T00:00:00Z", 0, 0),
        ("2038-01-19T03:14:07Z", 2_147_483_647, 0),
        ("2000-02-29T00:00:00Z", 951_782_400, 0),
    ] {
        assert_eq!(
            time::parse(input),
            Some(UNIX_EPOCH + Duration::new(seconds, nanos)),
            "{input}"
        );
    }
    assert_eq!(
        time::parse("2016-12-31T23:59:60Z"),
        Some(UNIX_EPOCH + Duration::new(1_483_228_799, 999_999_999))
    );
}

#[test]
fn rejects_non_fixed_or_invalid_server_times() {
    for input in [
        "",
        "2026-02-03T04:05:06",
        "2026-13-03T04:05:06Z",
        "2026-02-30T04:05:06Z",
        "1900-02-29T04:05:06Z",
        "2026-04-31T04:05:06Z",
        "2026-02-03T24:05:06Z",
        "2026-02-03T04:60:06Z",
        "2026-02-03T04:05:61Z",
        "202é-02-03T04:05:06Z",
        "2026-02-03T04:05:06.Z",
        "2026-02-03T04:05:06.1234567890Z",
        "2026/02-03T04:05:06Z",
        "2026-02/03T04:05:06Z",
        "2026-02-03 04:05:06Z",
        "2026-02-03T04-05:06Z",
        "2026-02-03T04:05-06Z",
    ] {
        assert_eq!(time::parse(input), None, "{input}");
    }
}

#[test]
fn accepts_the_last_day_of_thirty_day_months() {
    for (input, seconds) in [
        ("2026-04-30T00:00:00Z", 1_777_507_200),
        ("2026-06-30T00:00:00Z", 1_782_777_600),
        ("2026-09-30T00:00:00Z", 1_790_726_400),
        ("2026-11-30T00:00:00Z", 1_795_996_800),
    ] {
        assert_eq!(
            time::parse(input),
            Some(UNIX_EPOCH + Duration::from_secs(seconds)),
            "{input}"
        );
    }
}

#[test]
fn edge_dates_round_trip_at_millisecond_precision() {
    for input in [
        "1970-01-01T00:00:00.000Z",
        "2000-02-29T00:00:00.000Z",
        "2038-01-19T03:14:07.000Z",
        "2100-03-01T00:00:00.000Z",
        "9999-12-31T23:59:59.999Z",
    ] {
        let parsed = time::parse(input).unwrap();
        assert_eq!(time::format(parsed).as_deref(), Some(input), "{input}");
    }
}

proptest! {
    #[test]
    fn millisecond_aligned_times_round_trip(millis in 0_u64..=253_402_300_799_999) {
        let timestamp = UNIX_EPOCH + Duration::from_millis(millis);
        let formatted = time::format(timestamp).unwrap();
        prop_assert_eq!(time::parse(&formatted), Some(timestamp));
    }
}
