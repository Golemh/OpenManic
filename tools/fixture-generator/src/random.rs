//! Stable, dependency-free pseudo-random generation for fixtures.

use std::num::NonZeroU64;

/// A reproducible pseudo-random number generator for fixture data.
///
/// This is `SplitMix64` with its algorithm written out here as a compatibility
/// contract. Given the same seed, all supported OpenManic builds produce the
/// same sequence of [`next_u64`](Self::next_u64) results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeededPrng {
    state: u64,
}

impl SeededPrng {
    /// Creates a generator at the beginning of the sequence for `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next uniformly distributed 64-bit fixture value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    /// Returns a value in `0..upper_exclusive` without modulo bias.
    pub fn next_below(&mut self, upper_exclusive: NonZeroU64) -> u64 {
        let upper_exclusive = upper_exclusive.get();
        let rejection_threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        loop {
            let value = self.next_u64();
            if value >= rejection_threshold {
                return value % upper_exclusive;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SeededPrng;
    use std::num::NonZeroU64;

    #[test]
    fn same_seed_repeats_the_stable_sequence() {
        let mut first = SeededPrng::new(42);
        let mut second = SeededPrng::new(42);
        let values = (0..8).map(|_| first.next_u64()).collect::<Vec<_>>();
        let repeated = (0..8).map(|_| second.next_u64()).collect::<Vec<_>>();
        assert_eq!(values, repeated);
        assert_eq!(values[0], 13_679_457_532_755_275_413);
    }

    #[test]
    fn bounded_values_stay_inside_the_requested_range() {
        let mut generator = SeededPrng::new(7);
        let Some(upper_bound) = NonZeroU64::new(3) else {
            return;
        };
        for _ in 0..128 {
            assert!(generator.next_below(upper_bound) < upper_bound.get());
        }
    }

    #[test]
    fn zero_cannot_form_a_bounded_generation_request() {
        assert_eq!(NonZeroU64::new(0), None);
    }
}
