//! Boundary coverage for `IRCv3` server-time timestamps.

use std::time::{Duration, UNIX_EPOCH};

use papo_proto::{LineRef, ServerTime, time};
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
            Some(ServerTime::from_stored_millis(seconds * 1_000 + nanos / 1_000_000).unwrap()),
            "{input}"
        );
    }
    assert_eq!(
        time::parse("2016-12-31T23:59:60Z"),
        Some(ServerTime::from_stored_millis(1_483_228_799_999).unwrap())
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
            Some(ServerTime::from_stored_millis(seconds * 1_000).unwrap()),
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
        assert_eq!(parsed.to_string(), input, "{input}");
        assert_eq!(ServerTime::from_stored(input), Ok(parsed), "{input}");
        assert_eq!(
            ServerTime::from_stored_millis(parsed.as_millis()),
            Ok(parsed),
            "{input}"
        );
    }
}

#[test]
fn stored_forms_round_trip_before_epoch_and_at_bounds() {
    for input in [
        "0000-01-01T00:00:00.000Z",
        "1969-12-31T23:59:59.999Z",
        "9999-12-31T23:59:59.999Z",
    ] {
        let parsed: ServerTime = input.parse().unwrap();
        assert_eq!(parsed.to_string(), input);
        assert_eq!(ServerTime::from_stored(input), Ok(parsed));
        assert_eq!(
            ServerTime::from_stored_millis(parsed.as_millis()),
            Ok(parsed)
        );
        assert_eq!(
            ServerTime::from_stored_millis(
                parsed.as_millis() - i64::from(input.starts_with("0000"))
            )
            .is_err(),
            input.starts_with("0000")
        );
    }
}

#[test]
fn converts_server_times_to_exact_system_times() {
    let ergo = LineRef::parse(
        "@time=2024-01-02T03:04:05.678Z;msgid=GmU2V7xM :alice!~alice@cloak PRIVMSG #papo :hello",
    )
    .unwrap();
    let ergo_time = ergo.tag("time").unwrap();

    for (input, expected) in [
        (
            ergo_time.as_ref(),
            UNIX_EPOCH + Duration::new(1_704_164_645, 678_000_000),
        ),
        ("1970-01-01T00:00:00.000Z", UNIX_EPOCH),
        (
            "1969-12-31T23:59:59.999Z",
            UNIX_EPOCH - Duration::from_millis(1),
        ),
        (
            "9999-12-31T23:59:59.999Z",
            UNIX_EPOCH + Duration::new(253_402_300_799, 999_000_000),
        ),
    ] {
        assert_eq!(
            ServerTime::from_stored(input).unwrap().to_system_time(),
            expected,
            "{input}"
        );
    }
}

#[test]
fn formats_times_before_the_unix_epoch() {
    assert_eq!(
        time::format(ServerTime::from_stored_millis(-1).unwrap()),
        "1969-12-31T23:59:59.999Z"
    );
    assert_eq!(
        time::format(ServerTime::from_stored("1969-01-01T00:00:00.000Z").unwrap()),
        "1969-01-01T00:00:00.000Z"
    );
}

proptest! {
    #[test]
    fn millisecond_aligned_times_round_trip(millis in 0_u64..=253_402_300_799_999) {
        let timestamp = ServerTime::from_stored_millis(i64::try_from(millis).unwrap()).unwrap();
        let formatted = timestamp.to_string();
        prop_assert_eq!(time::parse(&formatted), Some(timestamp));
    }
}
