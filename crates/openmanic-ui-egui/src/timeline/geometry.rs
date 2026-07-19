//! Shared conversion between timeline UTC instants and horizontal pixels.

use core::fmt;

use openmanic_application::{IntervalIndex, TimelineInterval};
use openmanic_domain::{HalfOpenInterval, UtcMicros};

/// One validated horizontal transform for every timeline layer in an allocated region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineTransform {
    visible_range: HalfOpenInterval,
    x_start: f32,
    x_end: f32,
}

impl TimelineTransform {
    /// Creates a transform from one exact visible range and a positive finite pixel span.
    ///
    /// # Errors
    ///
    /// Returns [`TimelineTransformError`] when either coordinate is non-finite or the supplied
    /// width cannot define a positive finite horizontal span.
    pub fn try_new(
        visible_range: HalfOpenInterval,
        x_start: f32,
        width: f32,
    ) -> Result<Self, TimelineTransformError> {
        if !x_start.is_finite() {
            return Err(TimelineTransformError::NonFiniteStart { x_start });
        }
        if !width.is_finite() {
            return Err(TimelineTransformError::NonFiniteWidth { width });
        }
        if width <= 0.0 {
            return Err(TimelineTransformError::NonPositiveWidth { width });
        }
        let x_end = x_start + width;
        if !x_end.is_finite() || x_end <= x_start {
            return Err(TimelineTransformError::NonFiniteEnd { x_start, width });
        }
        Ok(Self {
            visible_range,
            x_start,
            x_end,
        })
    }

    /// Returns the exact UTC range represented by this transform.
    #[must_use]
    pub const fn visible_range(self) -> HalfOpenInterval {
        self.visible_range
    }

    /// Returns the inclusive pixel origin of the timeline region.
    #[must_use]
    pub const fn x_start(self) -> f32 {
        self.x_start
    }

    /// Returns the exclusive pixel boundary of the timeline region.
    #[must_use]
    pub const fn x_end(self) -> f32 {
        self.x_end
    }

    /// Returns the positive width of the timeline region in pixels.
    #[must_use]
    pub const fn width(self) -> f32 {
        self.x_end - self.x_start
    }

    /// Converts an instant into the shared horizontal coordinate system.
    ///
    /// Instants outside [`Self::visible_range`] intentionally remain outside the returned pixel
    /// span. Callers that require clipped geometry should use [`Self::range_geometry`].
    #[must_use]
    pub fn x_for(self, instant: UtcMicros) -> f32 {
        if instant == self.visible_range.start() {
            return self.x_start;
        }
        if instant == self.visible_range.end() {
            return self.x_end;
        }
        let elapsed = i128::from(instant.get()) - i128::from(self.visible_range.start().get());
        let x = self.x_start
            + time_ratio_as_pixel_fraction(elapsed, self.visible_range.duration_us())
                * self.width();
        if instant > self.visible_range.start() && instant < self.visible_range.end() {
            x.clamp(self.x_start, self.x_end.next_down())
        } else {
            x
        }
    }

    /// Converts a pixel inside the half-open timeline region to an exact UTC microsecond.
    ///
    /// The exclusive right edge maps to `None`, matching a half-open time range and preventing a
    /// pointer at the far boundary from selecting the preceding segment.
    #[must_use]
    pub fn time_at_x(self, x: f32) -> Option<UtcMicros> {
        if !x.is_finite() || x < self.x_start || x >= self.x_end {
            return None;
        }
        let ratio = f64::from(x - self.x_start) / f64::from(self.width());
        let elapsed = floored_duration_offset(ratio, self.visible_range.duration_us());
        let instant = i128::from(self.visible_range.start().get()) + i128::from(elapsed);
        if instant >= i128::from(self.visible_range.end().get()) {
            return None;
        }
        i64::try_from(instant).ok().map(UtcMicros::new)
    }

    /// Clips a time range to this transform and returns its exact horizontal geometry.
    #[must_use]
    pub fn range_geometry(self, range: HalfOpenInterval) -> Option<TimelineRangeGeometry> {
        let start = range.start().max(self.visible_range.start());
        let end = range.end().min(self.visible_range.end());
        let visible_range = HalfOpenInterval::try_new(start, end).ok()?;
        Some(TimelineRangeGeometry {
            source_range: range,
            visible_range,
            pixels: PixelRange::new(self.x_for(start), self.x_for(end)),
        })
    }
}

/// Failure while validating a [`TimelineTransform`] input span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimelineTransformError {
    /// The left coordinate was `NaN` or infinite.
    NonFiniteStart {
        /// The rejected coordinate.
        x_start: f32,
    },
    /// The width was `NaN` or infinite.
    NonFiniteWidth {
        /// The rejected width.
        width: f32,
    },
    /// The width was zero or negative.
    NonPositiveWidth {
        /// The rejected width.
        width: f32,
    },
    /// Adding the validated width to the start did not produce a finite positive end.
    NonFiniteEnd {
        /// The left coordinate.
        x_start: f32,
        /// The requested width.
        width: f32,
    },
}

impl fmt::Display for TimelineTransformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteStart { x_start } => {
                write!(formatter, "timeline x origin must be finite, got {x_start}")
            }
            Self::NonFiniteWidth { width } => {
                write!(formatter, "timeline width must be finite, got {width}")
            }
            Self::NonPositiveWidth { width } => {
                write!(formatter, "timeline width must be positive, got {width}")
            }
            Self::NonFiniteEnd { x_start, width } => write!(
                formatter,
                "timeline x origin {x_start} and width {width} do not produce a finite end"
            ),
        }
    }
}

impl std::error::Error for TimelineTransformError {}

/// One horizontal span in the common timeline coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelRange {
    start_x: f32,
    end_x: f32,
}

impl PixelRange {
    /// Creates a span whose boundaries were generated by a [`TimelineTransform`].
    #[must_use]
    pub const fn new(start_x: f32, end_x: f32) -> Self {
        Self { start_x, end_x }
    }

    /// Returns the left boundary in pixels.
    #[must_use]
    pub const fn start_x(self) -> f32 {
        self.start_x
    }

    /// Returns the right boundary in pixels.
    #[must_use]
    pub const fn end_x(self) -> f32 {
        self.end_x
    }

    /// Returns the span width in pixels.
    #[must_use]
    pub const fn width(self) -> f32 {
        self.end_x - self.start_x
    }
}

/// Exact clipped geometry reusable by selections, focus overlays, and future brackets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineRangeGeometry {
    source_range: HalfOpenInterval,
    visible_range: HalfOpenInterval,
    pixels: PixelRange,
}

impl TimelineRangeGeometry {
    /// Returns the original un-clipped range supplied by the caller.
    #[must_use]
    pub const fn source_range(self) -> HalfOpenInterval {
        self.source_range
    }

    /// Returns the portion visible in the transform's time range.
    #[must_use]
    pub const fn visible_range(self) -> HalfOpenInterval {
        self.visible_range
    }

    /// Returns the horizontal pixels calculated by the shared transform.
    #[must_use]
    pub const fn pixels(self) -> PixelRange {
        self.pixels
    }
}

/// Geometry for a schedule bracket, retaining its un-clipped occurrence range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduleBracketGeometry {
    range: TimelineRangeGeometry,
}

impl ScheduleBracketGeometry {
    /// Clips a schedule occurrence to the transform using the common time-to-pixel conversion.
    #[must_use]
    pub fn from_range(transform: TimelineTransform, occurrence: HalfOpenInterval) -> Option<Self> {
        transform
            .range_geometry(occurrence)
            .map(|range| Self { range })
    }

    /// Returns the shared range geometry for this bracket.
    #[must_use]
    pub const fn range(self) -> TimelineRangeGeometry {
        self.range
    }
}

/// Borrowed pixel geometry for one independently segmented presentation-band interval.
#[derive(Clone, Copy, Debug)]
pub struct BandSegmentGeometry<'a, T> {
    interval: &'a TimelineInterval<T>,
    range: TimelineRangeGeometry,
}

impl<'a, T> BandSegmentGeometry<'a, T> {
    /// Returns the original immutable presentation interval without cloning its raw fragments.
    #[must_use]
    pub const fn interval(self) -> &'a TimelineInterval<T> {
        self.interval
    }

    /// Returns the visible pixel geometry calculated from the shared transform.
    #[must_use]
    pub const fn range(self) -> TimelineRangeGeometry {
        self.range
    }
}

/// Builds visible pixel geometry for one independently segmented application projection band.
///
/// The interval index remains the authority for segment identity and raw fragments. This helper
/// merely clips the requested slice and applies the common transform; it never aligns or merges
/// the band with another band.
#[must_use]
pub fn band_geometry<T>(
    transform: TimelineTransform,
    index: &IntervalIndex<T>,
) -> Vec<BandSegmentGeometry<'_, T>> {
    index
        .intersecting(transform.visible_range())
        .iter()
        .filter_map(|interval| {
            transform
                .range_geometry(interval.range())
                .map(|range| BandSegmentGeometry { interval, range })
        })
        .collect()
}

/// Converts exact UTC duration arithmetic to the f32 fraction consumed by egui geometry.
///
/// f32 coordinates intentionally cannot retain microsecond precision over broad zoom ranges. The
/// raw [`UtcMicros`] values remain in the projection index and are used for the reverse hit test.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "egui's public coordinate space is f32; raw interval identity is retained separately"
)]
fn time_ratio_as_pixel_fraction(elapsed: i128, duration_us: u64) -> f32 {
    (elapsed as f64 / duration_us as f64) as f32
}

/// Converts a validated nonnegative in-range pixel ratio back to an integer microsecond offset.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller proves a finite ratio in [0, 1) and validates the result against the exclusive end"
)]
fn floored_duration_offset(ratio: f64, duration_us: u64) -> u64 {
    (ratio * duration_us as f64).floor() as u64
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{HalfOpenInterval, UtcMicros};

    use super::{ScheduleBracketGeometry, TimelineTransform, TimelineTransformError};

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("test ranges are positive")
    }

    #[test]
    fn transform_round_trips_expected_instants_at_required_widths_and_scales() {
        for logical_width in [720.0_f32, 1_024.0, 1_440.0] {
            assert_required_scale_geometry(logical_width);
        }
    }

    fn assert_required_scale_geometry(logical_width: f32) {
        for scale in [1.25_f32, 1.5, 1.75, 2.0] {
            let transform =
                TimelineTransform::try_new(range(0, 86_400_000), 8.0, logical_width * scale)
                    .expect("positive finite geometry");
            assert!(
                (transform.x_for(UtcMicros::new(0)) - transform.x_start()).abs() < f32::EPSILON
            );
            assert!(
                (transform.x_for(UtcMicros::new(86_400_000)) - transform.x_end()).abs()
                    < f32::EPSILON
            );
            assert_eq!(
                transform.time_at_x(transform.x_start()),
                Some(UtcMicros::new(0)),
                "width {logical_width}, scale {scale}"
            );
            assert_eq!(
                transform.time_at_x(transform.x_for(UtcMicros::new(43_200_000))),
                Some(UtcMicros::new(43_200_000)),
                "width {logical_width}, scale {scale}"
            );
            assert!(transform.time_at_x(transform.x_end()).is_none());
        }
    }

    #[test]
    fn range_and_schedule_geometry_clip_without_changing_the_original_boundaries() {
        let transform = TimelineTransform::try_new(range(100, 200), 20.0, 400.0)
            .expect("positive finite geometry");
        let geometry = transform
            .range_geometry(range(50, 150))
            .expect("the ranges overlap");
        assert_eq!(geometry.source_range(), range(50, 150));
        assert_eq!(geometry.visible_range(), range(100, 150));
        assert!((geometry.pixels().start_x() - 20.0).abs() < f32::EPSILON);
        assert!((geometry.pixels().end_x() - 220.0).abs() < f32::EPSILON);

        let bracket = ScheduleBracketGeometry::from_range(transform, range(150, 250))
            .expect("the occurrence overlaps");
        assert_eq!(bracket.range().visible_range(), range(150, 200));
        assert!((bracket.range().pixels().start_x() - 220.0).abs() < f32::EPSILON);
        assert!((bracket.range().pixels().end_x() - 420.0).abs() < f32::EPSILON);
    }

    #[test]
    fn right_edge_is_not_a_selectable_half_open_instant() {
        let transform =
            TimelineTransform::try_new(range(10, 20), 4.0, 80.0).expect("positive finite geometry");
        assert_eq!(transform.time_at_x(4.0), Some(UtcMicros::new(10)));
        assert_eq!(transform.time_at_x(84.0), None);
        assert_eq!(transform.time_at_x(3.999), None);
    }

    #[test]
    fn invalid_pixel_spans_are_rejected_without_a_fallback_transform() {
        assert!(matches!(
            TimelineTransform::try_new(range(0, 1), f32::NAN, 1.0),
            Err(TimelineTransformError::NonFiniteStart { .. })
        ));
        assert_eq!(
            TimelineTransform::try_new(range(0, 1), 0.0, 0.0),
            Err(TimelineTransformError::NonPositiveWidth { width: 0.0 })
        );
    }
}
