//! Adaptive timeline tick preparation without locale-specific label formatting.

use core::fmt;

use openmanic_domain::UtcMicros;

use super::TimelineTransform;

/// Validated pixel constraints used to select a non-overlapping timeline tick cadence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptiveTickLayout {
    estimated_label_width_px: f32,
    minimum_gap_px: f32,
}

impl AdaptiveTickLayout {
    /// Creates adaptive tick constraints from a renderer's measured label width and desired gap.
    ///
    /// The geometry layer deliberately accepts a measured width rather than formatting dates or
    /// making locale assumptions. The paint layer can therefore choose localized labels while
    /// this helper guarantees their estimated horizontal reservations do not overlap.
    ///
    /// # Errors
    ///
    /// Returns [`TickLayoutError`] when either measurement is non-finite, the label width is not
    /// positive, or the requested gap is negative.
    pub fn try_new(
        estimated_label_width_px: f32,
        minimum_gap_px: f32,
    ) -> Result<Self, TickLayoutError> {
        if !estimated_label_width_px.is_finite() {
            return Err(TickLayoutError::NonFiniteLabelWidth {
                width: estimated_label_width_px,
            });
        }
        if estimated_label_width_px <= 0.0 {
            return Err(TickLayoutError::NonPositiveLabelWidth {
                width: estimated_label_width_px,
            });
        }
        if !minimum_gap_px.is_finite() {
            return Err(TickLayoutError::NonFiniteGap {
                gap: minimum_gap_px,
            });
        }
        if minimum_gap_px < 0.0 {
            return Err(TickLayoutError::NegativeGap {
                gap: minimum_gap_px,
            });
        }
        Ok(Self {
            estimated_label_width_px,
            minimum_gap_px,
        })
    }

    /// Returns the measured label width reserved for each generated tick.
    #[must_use]
    pub const fn estimated_label_width_px(self) -> f32 {
        self.estimated_label_width_px
    }

    /// Returns the clear horizontal gap reserved between adjacent labels.
    #[must_use]
    pub const fn minimum_gap_px(self) -> f32 {
        self.minimum_gap_px
    }

    /// Generates evenly aligned tick instants and their shared-transform x coordinates.
    #[must_use]
    pub fn generate(self, transform: TimelineTransform) -> TickGeneration {
        let minimum_spacing_px = self.estimated_label_width_px + self.minimum_gap_px;
        let desired_step_us = desired_step_us(transform, minimum_spacing_px);
        let step_us = select_step_us(desired_step_us);
        let mut ticks = Vec::new();
        let step = i128::from(step_us);
        let start = i128::from(transform.visible_range().start().get());
        let end = i128::from(transform.visible_range().end().get());
        let mut instant = start.div_euclid(step) * step;
        if instant < start {
            instant += step;
        }
        while instant < end {
            let Ok(value) = i64::try_from(instant) else {
                break;
            };
            let tick_instant = UtcMicros::new(value);
            ticks.push(TimelineTick {
                instant: tick_instant,
                x: transform.x_for(tick_instant),
            });
            instant += step;
        }
        TickGeneration { step_us, ticks }
    }
}

/// Failure while validating adaptive tick measurements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TickLayoutError {
    /// The estimated label width was `NaN` or infinite.
    NonFiniteLabelWidth {
        /// The rejected measurement.
        width: f32,
    },
    /// The estimated label width was zero or negative.
    NonPositiveLabelWidth {
        /// The rejected measurement.
        width: f32,
    },
    /// The minimum gap was `NaN` or infinite.
    NonFiniteGap {
        /// The rejected measurement.
        gap: f32,
    },
    /// The minimum gap was negative.
    NegativeGap {
        /// The rejected measurement.
        gap: f32,
    },
}

impl fmt::Display for TickLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteLabelWidth { width } => {
                write!(
                    formatter,
                    "timeline tick label width must be finite, got {width}"
                )
            }
            Self::NonPositiveLabelWidth { width } => {
                write!(
                    formatter,
                    "timeline tick label width must be positive, got {width}"
                )
            }
            Self::NonFiniteGap { gap } => {
                write!(
                    formatter,
                    "timeline tick label gap must be finite, got {gap}"
                )
            }
            Self::NegativeGap { gap } => {
                write!(
                    formatter,
                    "timeline tick label gap must not be negative, got {gap}"
                )
            }
        }
    }
}

impl std::error::Error for TickLayoutError {}

/// One tick aligned in UTC microseconds and the common timeline pixel space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineTick {
    instant: UtcMicros,
    x: f32,
}

impl TimelineTick {
    /// Returns the exact UTC instant represented by the tick.
    #[must_use]
    pub const fn instant(self) -> UtcMicros {
        self.instant
    }

    /// Returns the horizontal coordinate from the common timeline transform.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }
}

/// A generated cadence and the bounded tick list in the visible half-open range.
#[derive(Clone, Debug, PartialEq)]
pub struct TickGeneration {
    step_us: u64,
    ticks: Vec<TimelineTick>,
}

impl TickGeneration {
    /// Returns the selected cadence in microseconds.
    #[must_use]
    pub const fn step_us(&self) -> u64 {
        self.step_us
    }

    /// Returns each aligned tick in increasing time order.
    #[must_use]
    pub fn ticks(&self) -> &[TimelineTick] {
        &self.ticks
    }
}

const SECOND_US: u64 = 1_000_000;
const MINUTE_US: u64 = 60 * SECOND_US;
const HOUR_US: u64 = 60 * MINUTE_US;
const DAY_US: u64 = 24 * HOUR_US;
const YEAR_US: u64 = 365 * DAY_US;

const PREFERRED_STEPS_US: &[u64] = &[
    SECOND_US,
    5 * SECOND_US,
    10 * SECOND_US,
    15 * SECOND_US,
    30 * SECOND_US,
    MINUTE_US,
    2 * MINUTE_US,
    5 * MINUTE_US,
    10 * MINUTE_US,
    15 * MINUTE_US,
    30 * MINUTE_US,
    HOUR_US,
    2 * HOUR_US,
    3 * HOUR_US,
    6 * HOUR_US,
    12 * HOUR_US,
    DAY_US,
    2 * DAY_US,
    7 * DAY_US,
    14 * DAY_US,
    30 * DAY_US,
    90 * DAY_US,
    180 * DAY_US,
    YEAR_US,
    2 * YEAR_US,
    5 * YEAR_US,
    10 * YEAR_US,
];

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "tick cadence is deliberately chosen from f32 paint measurements and bounded to u64 microseconds"
)]
fn desired_step_us(transform: TimelineTransform, minimum_spacing_px: f32) -> u64 {
    let duration = transform.visible_range().duration_us();
    let desired =
        (duration as f64 * f64::from(minimum_spacing_px) / f64::from(transform.width())).ceil();
    if desired >= u64::MAX as f64 {
        u64::MAX
    } else {
        desired as u64
    }
}

fn select_step_us(desired_step_us: u64) -> u64 {
    if let Some(step) = PREFERRED_STEPS_US
        .iter()
        .copied()
        .find(|step| *step >= desired_step_us)
    {
        return step;
    }
    let largest = *PREFERRED_STEPS_US.last().unwrap_or(&YEAR_US);
    largest.saturating_mul(ceil_div(desired_step_us, largest))
}

fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    numerator / denominator + u64::from(!numerator.is_multiple_of(denominator))
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{HalfOpenInterval, UtcMicros};

    use super::{AdaptiveTickLayout, TickLayoutError};
    use crate::timeline::TimelineTransform;

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("test ranges are positive")
    }

    #[test]
    fn adaptive_ticks_stay_non_overlapping_at_required_widths_and_scales() {
        let layout = AdaptiveTickLayout::try_new(72.0, 8.0).expect("valid tick layout");
        for logical_width in [720.0_f32, 1_024.0, 1_440.0] {
            assert_required_scale_ticks(layout, logical_width);
        }
    }

    fn assert_required_scale_ticks(layout: AdaptiveTickLayout, logical_width: f32) {
        for scale in [1.25_f32, 1.5, 1.75, 2.0] {
            let transform = TimelineTransform::try_new(
                range(0, 24 * 60 * 60 * 1_000_000),
                0.0,
                logical_width * scale,
            )
            .expect("positive finite geometry");
            let generation = layout.generate(transform);
            assert!(!generation.ticks().is_empty());
            assert_tick_spacing(layout, &generation, logical_width, scale);
        }
    }

    fn assert_tick_spacing(
        layout: AdaptiveTickLayout,
        generation: &super::TickGeneration,
        logical_width: f32,
        scale: f32,
    ) {
        for pair in generation.ticks().windows(2) {
            let distance = pair[1].x() - pair[0].x();
            assert!(
                distance + 0.001 >= layout.estimated_label_width_px() + layout.minimum_gap_px(),
                "width {logical_width}, scale {scale}, distance {distance}"
            );
        }
    }

    #[test]
    fn ticks_are_aligned_sorted_and_stay_inside_the_half_open_range() {
        let layout = AdaptiveTickLayout::try_new(60.0, 12.0).expect("valid tick layout");
        let transform = TimelineTransform::try_new(range(-10_000_000, 20_000_000), 10.0, 180.0)
            .expect("positive finite geometry");
        let generation = layout.generate(transform);
        let ticks = generation.ticks();
        assert!(
            ticks
                .windows(2)
                .all(|pair| pair[0].instant() < pair[1].instant())
        );
        assert!(ticks.iter().all(|tick| {
            tick.instant() >= transform.visible_range().start()
                && tick.instant() < transform.visible_range().end()
                && i128::from(tick.instant().get()).rem_euclid(i128::from(generation.step_us()))
                    == 0
        }));
    }

    #[test]
    fn invalid_tick_measurements_are_rejected_without_overlapping_fallbacks() {
        assert_eq!(
            AdaptiveTickLayout::try_new(0.0, 4.0),
            Err(TickLayoutError::NonPositiveLabelWidth { width: 0.0 })
        );
        assert!(matches!(
            AdaptiveTickLayout::try_new(f32::NAN, 4.0),
            Err(TickLayoutError::NonFiniteLabelWidth { .. })
        ));
        assert_eq!(
            AdaptiveTickLayout::try_new(30.0, -1.0),
            Err(TickLayoutError::NegativeGap { gap: -1.0 })
        );
    }
}
