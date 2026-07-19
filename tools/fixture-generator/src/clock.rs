//! Manually controlled UTC and monotonic fixture time.

/// A UTC timestamp represented in microseconds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct UtcMicros(i64);

impl UtcMicros {
    /// Creates a UTC timestamp from its microsecond representation.
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the timestamp as UTC microseconds.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A monotonic tick count for elapsed-time fixture assertions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicTicks(u64);

impl MonotonicTicks {
    /// Creates a monotonic tick count.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the number of monotonic ticks.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A deterministic clock whose UTC and monotonic axes advance independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManualClock {
    utc_micros: UtcMicros,
    monotonic_ticks: MonotonicTicks,
}

impl ManualClock {
    /// Creates a clock at the supplied UTC timestamp and monotonic tick count.
    #[must_use]
    pub const fn new(utc_micros: UtcMicros, monotonic_ticks: MonotonicTicks) -> Self {
        Self {
            utc_micros,
            monotonic_ticks,
        }
    }

    /// Returns the current UTC timestamp.
    #[must_use]
    pub const fn utc_micros(self) -> UtcMicros {
        self.utc_micros
    }

    /// Returns the current monotonic tick count.
    #[must_use]
    pub const fn monotonic_ticks(self) -> MonotonicTicks {
        self.monotonic_ticks
    }

    /// Advances only the UTC timeline by a non-negative duration.
    pub fn advance_utc_micros(&mut self, microseconds: u64) {
        let duration = i64::try_from(microseconds).unwrap_or(i64::MAX);
        self.utc_micros = UtcMicros(self.utc_micros.0.saturating_add(duration));
    }

    /// Sets UTC independently, including backwards jumps used by recovery tests.
    pub fn set_utc_micros(&mut self, utc_micros: UtcMicros) {
        self.utc_micros = utc_micros;
    }

    /// Advances only the monotonic timeline by `ticks`.
    pub fn advance_monotonic_ticks(&mut self, ticks: u64) {
        self.monotonic_ticks = MonotonicTicks(self.monotonic_ticks.0.saturating_add(ticks));
    }
}

#[cfg(test)]
mod tests {
    use super::{ManualClock, MonotonicTicks, UtcMicros};

    #[test]
    fn utc_and_monotonic_time_advance_independently() {
        let mut clock = ManualClock::new(UtcMicros::new(100), MonotonicTicks::new(4));
        clock.set_utc_micros(UtcMicros::new(75));
        assert_eq!(clock.utc_micros(), UtcMicros::new(75));
        assert_eq!(clock.monotonic_ticks(), MonotonicTicks::new(4));
        clock.advance_monotonic_ticks(9);
        assert_eq!(clock.utc_micros(), UtcMicros::new(75));
        assert_eq!(clock.monotonic_ticks(), MonotonicTicks::new(13));
        clock.advance_utc_micros(25);
        assert_eq!(clock.utc_micros(), UtcMicros::new(100));
    }

    #[test]
    fn advances_saturate_at_numeric_boundaries() {
        let mut clock = ManualClock::new(UtcMicros::new(i64::MAX), MonotonicTicks::new(u64::MAX));
        clock.advance_utc_micros(1);
        clock.advance_monotonic_ticks(1);
        assert_eq!(clock.utc_micros(), UtcMicros::new(i64::MAX));
        assert_eq!(clock.monotonic_ticks(), MonotonicTicks::new(u64::MAX));
    }
}
