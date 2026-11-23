//! Day ranges for Today and Calendar, as UTC days.
//!
//! "Now" comes in as a parameter so tests can pin a day without a clock.

use std::time::{SystemTime, UNIX_EPOCH};

use openmanic_domain::{HalfOpenInterval, UtcMicros};

/// Microseconds in one UTC day.
const DAY_US: i64 = 86_400_000_000;

/// Microseconds since the Unix epoch; 0 for a clock set before 1970.
pub(crate) fn utc_now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
        })
}

/// The UTC day `offset_days` away from the day holding `now_us`.
pub(crate) fn day_range(offset_days: i32, now_us: i64) -> Option<HalfOpenInterval> {
    let day_start = now_us.div_euclid(DAY_US).saturating_mul(DAY_US);
    let offset = i64::from(offset_days).saturating_mul(DAY_US);
    let start = day_start.saturating_add(offset);
    HalfOpenInterval::try_new(
        UtcMicros::new(start),
        UtcMicros::new(start.saturating_add(DAY_US)),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::{DAY_US, day_range, utc_now_micros};

    // 2026-03-08 00:00:00 UTC
    const MIDNIGHT_US: i64 = 1_772_928_000_000_000;

    fn start_and_end(offset_days: i32, now_us: i64) -> (i64, i64) {
        let range = day_range(offset_days, now_us).expect("a day in range has an interval");
        (range.start().get(), range.end().get())
    }

    #[test]
    fn last_microsecond_before_midnight_is_the_previous_day() {
        assert_eq!(
            start_and_end(0, MIDNIGHT_US - 1),
            (MIDNIGHT_US - DAY_US, MIDNIGHT_US)
        );
    }

    #[test]
    fn midnight_starts_the_new_day() {
        assert_eq!(
            start_and_end(0, MIDNIGHT_US),
            (MIDNIGHT_US, MIDNIGHT_US + DAY_US)
        );
    }

    #[test]
    fn offsets_move_whole_days() {
        let noon_us = MIDNIGHT_US + DAY_US / 2;
        assert_eq!(
            start_and_end(-1, noon_us),
            (MIDNIGHT_US - DAY_US, MIDNIGHT_US)
        );
        assert_eq!(
            start_and_end(1, noon_us),
            (MIDNIGHT_US + DAY_US, MIDNIGHT_US + 2 * DAY_US)
        );
    }

    #[test]
    fn the_clock_reads_after_this_code_was_written() {
        // 2026-01-01 00:00:00 UTC; a host clock earlier than this is broken
        assert!(utc_now_micros() > 1_767_225_600_000_000);
    }

    #[test]
    fn a_time_before_the_epoch_rounds_down() {
        assert_eq!(start_and_end(0, -1), (-DAY_US, 0));
    }
}
