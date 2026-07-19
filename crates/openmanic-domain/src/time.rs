//! Signed UTC-microsecond instants and positive half-open temporal ranges.

use core::fmt;

/// A signed count of microseconds from the Unix epoch.
///
/// This is a persisted wall-clock instant. Runtime monotonic clocks remain outside the domain
/// because they cannot survive a process restart.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UtcMicros(i64);

impl UtcMicros {
    /// Creates an instant from its exact signed microsecond representation.
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the exact signed microsecond representation.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Adds a signed number of microseconds without silently overflowing.
    ///
    /// # Errors
    ///
    /// Returns [`UtcMicrosArithmeticError::Overflow`] when the result is outside `i64`.
    pub fn checked_add(self, microseconds: i64) -> Result<Self, UtcMicrosArithmeticError> {
        self.0
            .checked_add(microseconds)
            .map(Self)
            .ok_or(UtcMicrosArithmeticError::Overflow)
    }

    /// Subtracts a signed number of microseconds without silently overflowing.
    ///
    /// # Errors
    ///
    /// Returns [`UtcMicrosArithmeticError::Overflow`] when the result is outside `i64`.
    pub fn checked_sub(self, microseconds: i64) -> Result<Self, UtcMicrosArithmeticError> {
        self.0
            .checked_sub(microseconds)
            .map(Self)
            .ok_or(UtcMicrosArithmeticError::Overflow)
    }

    /// Calculates a signed difference without silently overflowing.
    ///
    /// # Errors
    ///
    /// Returns [`UtcMicrosArithmeticError::Overflow`] when the signed difference is outside
    /// `i64`.
    pub fn checked_difference(self, earlier: Self) -> Result<i64, UtcMicrosArithmeticError> {
        self.0
            .checked_sub(earlier.0)
            .ok_or(UtcMicrosArithmeticError::Overflow)
    }
}

/// Failure from checked UTC-microsecond arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UtcMicrosArithmeticError {
    /// The requested operation cannot be represented in a signed 64-bit microsecond count.
    Overflow,
}

impl fmt::Display for UtcMicrosArithmeticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UTC microsecond arithmetic overflowed i64")
    }
}

impl std::error::Error for UtcMicrosArithmeticError {}

/// A positive half-open interval `[start, end)` between UTC instants.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HalfOpenInterval {
    start: UtcMicros,
    end: UtcMicros,
}

impl HalfOpenInterval {
    /// Creates a positive half-open interval.
    ///
    /// # Errors
    ///
    /// Returns [`HalfOpenIntervalError::NotPositive`] unless `end` is strictly after `start`.
    pub fn try_new(start: UtcMicros, end: UtcMicros) -> Result<Self, HalfOpenIntervalError> {
        if end <= start {
            return Err(HalfOpenIntervalError::NotPositive { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive start boundary.
    #[must_use]
    pub const fn start(self) -> UtcMicros {
        self.start
    }

    /// Returns the exclusive end boundary.
    #[must_use]
    pub const fn end(self) -> UtcMicros {
        self.end
    }

    /// Returns the positive duration as an unsigned count of microseconds.
    #[must_use]
    pub fn duration_us(self) -> u64 {
        let difference = i128::from(self.end.get()) - i128::from(self.start.get());
        u64::try_from(difference).unwrap_or(u64::MAX)
    }

    /// Returns true when this interval ends exactly where `next` begins.
    #[must_use]
    pub fn is_immediately_before(self, next: Self) -> bool {
        self.end == next.start
    }

    /// Returns true when either interval is immediately before the other.
    #[must_use]
    pub fn is_adjacent_to(self, other: Self) -> bool {
        self.end == other.start || other.end == self.start
    }

    /// Returns true when the two positive half-open intervals share at least one instant.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Failure while constructing a positive half-open interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalfOpenIntervalError {
    /// The exclusive end was not strictly after the inclusive start.
    NotPositive {
        /// Requested inclusive start.
        start: UtcMicros,
        /// Requested exclusive end.
        end: UtcMicros,
    },
}

impl fmt::Display for HalfOpenIntervalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPositive { start, end } => write!(
                formatter,
                "half-open interval must have end after start, got [{}, {})",
                start.get(),
                end.get()
            ),
        }
    }
}

impl std::error::Error for HalfOpenIntervalError {}

#[cfg(test)]
mod tests {
    use super::{HalfOpenInterval, HalfOpenIntervalError, UtcMicros, UtcMicrosArithmeticError};

    fn instant(value: i64) -> UtcMicros {
        UtcMicros::new(value)
    }

    #[test]
    fn utc_microseconds_use_checked_signed_arithmetic() {
        assert_eq!(instant(-4).checked_add(9), Ok(instant(5)));
        assert_eq!(instant(5).checked_sub(9), Ok(instant(-4)));
        assert_eq!(instant(5).checked_difference(instant(-4)), Ok(9));
        assert_eq!(
            instant(i64::MAX).checked_add(1),
            Err(UtcMicrosArithmeticError::Overflow)
        );
        assert_eq!(
            instant(i64::MIN).checked_difference(instant(i64::MAX)),
            Err(UtcMicrosArithmeticError::Overflow)
        );
    }

    #[test]
    fn interval_rejects_empty_and_reversed_boundaries() {
        assert_eq!(
            HalfOpenInterval::try_new(instant(3), instant(3)),
            Err(HalfOpenIntervalError::NotPositive {
                start: instant(3),
                end: instant(3),
            })
        );
        assert!(HalfOpenInterval::try_new(instant(4), instant(3)).is_err());
    }

    #[test]
    fn half_open_interval_adjacency_and_overlap_hold_at_boundaries() {
        let left = HalfOpenInterval::try_new(instant(0), instant(10)).expect("positive");
        let adjacent = HalfOpenInterval::try_new(instant(10), instant(20)).expect("positive");
        let overlapping = HalfOpenInterval::try_new(instant(9), instant(20)).expect("positive");
        assert!(left.is_immediately_before(adjacent));
        assert!(left.is_adjacent_to(adjacent));
        assert!(!left.overlaps(adjacent));
        assert!(left.overlaps(overlapping));
    }

    #[test]
    fn small_boundary_matrix_matches_half_open_overlap_definition() {
        let intervals = positive_small_intervals();
        for &(start, end, interval) in &intervals {
            for &(other_start, other_end, other) in &intervals {
                let expected = start < other_end && other_start < end;
                assert_eq!(
                    interval.overlaps(other),
                    expected,
                    "[{start}, {end}) versus [{other_start}, {other_end})"
                );
            }
        }
    }

    fn positive_small_intervals() -> Vec<(i64, i64, HalfOpenInterval)> {
        let mut values = Vec::new();
        for start in -3..=3 {
            for end in start + 1..=4 {
                values.push((
                    start,
                    end,
                    HalfOpenInterval::try_new(instant(start), instant(end)).expect("positive"),
                ));
            }
        }
        values
    }
}
