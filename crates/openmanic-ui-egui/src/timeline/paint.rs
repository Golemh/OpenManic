//! Deterministic, paint-only preparation for the three timeline bands.
//!
//! The projection remains immutable and authoritative. Dense preparation merely groups intervals
//! that cannot be distinguished at the current pixel scale; exact raw fragments stay attached to
//! the immutable source intervals for hit testing, hover, and selection.

use std::collections::BTreeMap;

use openmanic_application::{
    ActivityStateValue, ApplicationBandValue, CategoryBandValue, IntervalIndex, TimelineInterval,
    TimelineSnapshot, TimelineTotals,
};
use openmanic_domain::HalfOpenInterval;

use super::{PixelRange, ScheduleBracketGeometry, TimelineRangeGeometry, TimelineTransform};

const DENSE_SEGMENT_MAX_WIDTH_PX: f32 = 1.0;

/// Whether a segment has a visible fill in the paint layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintFill {
    /// The renderer may apply its value-specific fill.
    Visible,
    /// The geometry remains present for selection and hover but has no fill.
    Absent,
}

/// Exact visible geometry for one immutable source presentation interval.
#[derive(Debug)]
pub struct PaintSegment<'a, T> {
    interval: &'a TimelineInterval<T>,
    geometry: TimelineRangeGeometry,
    fill: PaintFill,
}

impl<T> Copy for PaintSegment<'_, T> {}

impl<T> Clone for PaintSegment<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> PaintSegment<'a, T> {
    /// Returns the original presentation interval, including all exact raw fragments.
    #[must_use]
    pub const fn interval(self) -> &'a TimelineInterval<T> {
        self.interval
    }

    /// Returns common-transform geometry without changing the source boundaries.
    #[must_use]
    pub const fn geometry(self) -> TimelineRangeGeometry {
        self.geometry
    }

    /// Returns whether normal painting should fill this segment.
    #[must_use]
    pub const fn fill(self) -> PaintFill {
        self.fill
    }
}

/// One deterministic pixel-column aggregation that retains every source segment.
#[derive(Clone, Debug)]
pub struct DensePaintBin<'a, T> {
    pixels: PixelRange,
    sources: Vec<PaintSegment<'a, T>>,
}

impl<'a, T> DensePaintBin<'a, T> {
    /// Returns the one pixel-column geometry to paint for this dense bin.
    #[must_use]
    pub const fn pixels(&self) -> PixelRange {
        self.pixels
    }

    /// Returns immutable source segments retained for exact inspection and palette composition.
    #[must_use]
    pub fn sources(&self) -> &[PaintSegment<'a, T>] {
        &self.sources
    }
}

/// One painter work item. Aggregated bins are painted once, never as per-interval widgets.
#[derive(Clone, Debug)]
pub enum PaintPrimitive<'a, T> {
    /// A segment wide enough to retain exact paint geometry.
    Segment(PaintSegment<'a, T>),
    /// Several sub-pixel source segments sharing one deterministic paint column.
    Dense(DensePaintBin<'a, T>),
}

impl<T> PaintPrimitive<'_, T> {
    /// Returns the left coordinate used to preserve deterministic painter order.
    #[must_use]
    pub fn start_x(&self) -> f32 {
        match self {
            Self::Segment(segment) => segment.geometry().pixels().start_x(),
            Self::Dense(bin) => bin.pixels().start_x(),
        }
    }
}

/// Paint-only representation for one independently segmented timeline band.
#[derive(Clone, Debug)]
pub struct PaintBand<'a, T> {
    source_intervals: &'a [TimelineInterval<T>],
    primitives: Vec<PaintPrimitive<'a, T>>,
}

impl<'a, T> PaintBand<'a, T> {
    /// Returns every original immutable presentation interval, unchanged by aggregation.
    #[must_use]
    pub const fn source_intervals(&self) -> &'a [TimelineInterval<T>] {
        self.source_intervals
    }

    /// Returns the bounded painter work items in deterministic horizontal order.
    #[must_use]
    pub fn primitives(&self) -> &[PaintPrimitive<'a, T>] {
        &self.primitives
    }

    /// Returns the original interval count retained independently of painter aggregation.
    #[must_use]
    pub const fn raw_interval_count(&self) -> usize {
        self.source_intervals.len()
    }
}

/// Activity-band paint data, including explicit no-fill Powered Off geometry.
pub type ActivityPaintBand<'a> = PaintBand<'a, ActivityStateValue>;

/// Paint-only work for all three independently segmented bands sharing one transform.
#[derive(Clone, Debug)]
pub struct TimelinePaintPlan<'a> {
    transform: TimelineTransform,
    totals: TimelineTotals,
    category: PaintBand<'a, CategoryBandValue>,
    activity: ActivityPaintBand<'a>,
    application: PaintBand<'a, ApplicationBandValue>,
}

impl<'a> TimelinePaintPlan<'a> {
    /// Prepares all bands from one immutable correlated snapshot and one shared transform.
    #[must_use]
    pub fn from_snapshot(transform: TimelineTransform, snapshot: &'a TimelineSnapshot) -> Self {
        Self {
            transform,
            totals: snapshot.totals(),
            category: prepare_band(transform, snapshot.category_band(), |_| PaintFill::Visible),
            activity: prepare_band(transform, snapshot.activity_band(), |value| {
                if *value == ActivityStateValue::PoweredOff {
                    PaintFill::Absent
                } else {
                    PaintFill::Visible
                }
            }),
            application: prepare_band(transform, snapshot.application_band(), |_| {
                PaintFill::Visible
            }),
        }
    }

    /// Returns the sole transform shared by every prepared band and future overlay.
    #[must_use]
    pub const fn transform(&self) -> TimelineTransform {
        self.transform
    }

    /// Returns the source totals unchanged by paint-time aggregation.
    #[must_use]
    pub const fn totals(&self) -> TimelineTotals {
        self.totals
    }

    /// Returns paint work for the category band.
    #[must_use]
    pub const fn category(&self) -> &PaintBand<'a, CategoryBandValue> {
        &self.category
    }

    /// Returns paint work for the activity band.
    #[must_use]
    pub const fn activity(&self) -> &ActivityPaintBand<'a> {
        &self.activity
    }

    /// Returns paint work for the application band.
    #[must_use]
    pub const fn application(&self) -> &PaintBand<'a, ApplicationBandValue> {
        &self.application
    }
}

/// Geometry hook for a caller-owned schedule occurrence.
///
/// The hook deliberately borrows the caller's occurrence rather than manufacturing an occurrence
/// identity before the authoritative schedule projection exists.
#[derive(Clone, Copy, Debug)]
pub struct ScheduleOverlayGeometry<'a, T> {
    occurrence: &'a T,
    bracket: ScheduleBracketGeometry,
}

impl<'a, T> ScheduleOverlayGeometry<'a, T> {
    /// Returns the original caller-owned occurrence identity/value.
    #[must_use]
    pub const fn occurrence(&self) -> &'a T {
        self.occurrence
    }

    /// Returns the clipped bracket geometry calculated by the shared transform.
    #[must_use]
    pub const fn bracket(&self) -> ScheduleBracketGeometry {
        self.bracket
    }
}

/// Prepares schedule brackets using the same transform as every timeline band.
///
/// Values are borrowed and only their supplied ranges are inspected, so this function neither
/// allocates schedule identities nor alters recorded activity.
#[must_use]
pub fn prepare_schedule_overlays<'a, T>(
    transform: TimelineTransform,
    occurrences: impl IntoIterator<Item = (&'a T, HalfOpenInterval)>,
) -> Vec<ScheduleOverlayGeometry<'a, T>> {
    occurrences
        .into_iter()
        .filter_map(|(occurrence, range)| {
            ScheduleBracketGeometry::from_range(transform, range).map(|bracket| {
                ScheduleOverlayGeometry {
                    occurrence,
                    bracket,
                }
            })
        })
        .collect()
}

fn prepare_band<'a, T>(
    transform: TimelineTransform,
    index: &'a IntervalIndex<T>,
    fill_for: impl Fn(&T) -> PaintFill,
) -> PaintBand<'a, T> {
    let source_intervals = index.intersecting(transform.visible_range());
    let mut exact = Vec::new();
    let mut dense = BTreeMap::<i64, Vec<PaintSegment<'a, T>>>::new();
    for interval in source_intervals {
        let Some(geometry) = transform.range_geometry(interval.range()) else {
            continue;
        };
        let segment = PaintSegment {
            interval,
            geometry,
            fill: fill_for(interval.value()),
        };
        if geometry.pixels().width() <= DENSE_SEGMENT_MAX_WIDTH_PX {
            dense
                .entry(pixel_column(geometry.pixels().start_x()))
                .or_default()
                .push(segment);
        } else {
            exact.push(PaintPrimitive::Segment(segment));
        }
    }
    exact.extend(dense.into_iter().map(|(column, sources)| {
        PaintPrimitive::Dense(DensePaintBin {
            pixels: pixel_column_range(transform, column),
            sources,
        })
    }));
    exact.sort_by(|left, right| left.start_x().total_cmp(&right.start_x()));
    PaintBand {
        source_intervals,
        primitives: exact,
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "the finite timeline coordinate is intentionally grouped into a bounded logical pixel column"
)]
fn pixel_column(x: f32) -> i64 {
    let floored = x.floor();
    if floored <= i64::MIN as f32 {
        i64::MIN
    } else if floored >= i64::MAX as f32 {
        i64::MAX
    } else {
        floored as i64
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "paint bins intentionally use f32 egui coordinates after integer column grouping"
)]
fn pixel_column_range(transform: TimelineTransform, column: i64) -> PixelRange {
    let start = (column as f32).max(transform.x_start());
    let end = ((column.saturating_add(1)) as f32).min(transform.x_end());
    PixelRange::new(start, end.max(start))
}

#[cfg(test)]
mod tests {
    use openmanic_application::{
        DataRevision, ProjectionContextKey, TimelineContext, TimelineProjectionSource,
        TimelineProjector, TimelineRawIntervalId, TimelineSourceActivity,
    };
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, HalfOpenInterval,
        PowerTransitionEvidence, TrackerRunId, UtcMicros,
    };

    use super::{PaintFill, PaintPrimitive, TimelinePaintPlan, prepare_schedule_overlays};
    use crate::timeline::TimelineTransform;

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("positive fixture range")
    }

    fn activity(raw_id: u64, start: i64, end: i64, state: ActivityState) -> TimelineSourceActivity {
        let evidence = match state {
            ActivityState::PoweredOff => ActivityEvidence::confirmed_shutdown(
                PowerTransitionEvidence::try_new(UtcMicros::new(start), UtcMicros::new(end))
                    .expect("positive power fixture"),
            ),
            _ => ActivityEvidence::try_from_cause(ActivityCause::IdleThreshold)
                .expect("ordinary evidence fixture"),
        };
        TimelineSourceActivity::new(
            TimelineRawIntervalId::new(raw_id),
            ActivityInterval::try_new(
                TrackerRunId::from_bytes([9; 16]),
                range(start, end),
                state,
                evidence,
                None,
            )
            .expect("valid activity fixture"),
        )
    }

    fn with_plan(
        activities: &[TimelineSourceActivity],
        range: HalfOpenInterval,
        inspect: impl FnOnce(&TimelinePaintPlan<'_>),
    ) {
        let snapshot = TimelineProjector::build(
            TimelineContext::new(ProjectionContextKey::new(1), range),
            TimelineProjectionSource::new(DataRevision::new(1), activities, &[]),
        )
        .expect("ordered fixture projects");
        let plan = TimelinePaintPlan::from_snapshot(
            TimelineTransform::try_new(range, 0.0, 100.0).expect("valid paint transform"),
            &snapshot,
        );
        inspect(&plan);
    }

    #[test]
    fn dense_10000_interval_fixture_is_binned_without_replacing_raw_records_or_totals() {
        let activities: Vec<_> = (0_i64..10_000)
            .map(|index| {
                activity(
                    index.cast_unsigned(),
                    index * 10,
                    (index + 1) * 10,
                    if index % 2 == 0 {
                        ActivityState::Idle
                    } else {
                        ActivityState::Unavailable
                    },
                )
            })
            .collect();
        with_plan(&activities, range(0, 100_000), |plan| {
            assert_eq!(plan.activity().raw_interval_count(), 10_000);
            assert_eq!(plan.activity().source_intervals().len(), 10_000);
            assert_eq!(plan.totals().visible_duration_us(), 100_000);
            assert!(plan.activity().primitives().len() <= 100);
            assert!(
                plan.activity()
                    .primitives()
                    .iter()
                    .all(|primitive| matches!(primitive, PaintPrimitive::Dense(_)))
            );
        });
    }

    #[test]
    fn powered_off_has_geometry_and_raw_identity_but_no_fill() {
        let activities = [
            activity(10, 0, 50, ActivityState::Idle),
            activity(11, 50, 100, ActivityState::PoweredOff),
        ];
        with_plan(&activities, range(0, 100), |plan| {
            let powered_off = plan
                .activity()
                .primitives()
                .iter()
                .find_map(|primitive| match primitive {
                    PaintPrimitive::Segment(segment)
                        if segment.interval().value().state() == ActivityState::PoweredOff =>
                    {
                        Some(*segment)
                    }
                    _ => None,
                })
                .expect("powered-off interval retains a paint segment");
            assert_eq!(powered_off.fill(), PaintFill::Absent);
            assert_eq!(powered_off.interval().raw_fragments()[0].raw_id().get(), 11);
            assert_eq!(powered_off.geometry().visible_range(), range(50, 100));
        });
    }

    #[test]
    fn schedule_hook_uses_the_same_transform_without_creating_an_identity() {
        let marker = "caller-owned occurrence";
        let transform = TimelineTransform::try_new(range(0, 100), 10.0, 100.0)
            .expect("valid schedule transform");
        let overlays = prepare_schedule_overlays(transform, [(&marker, range(-20, 30))]);
        assert_eq!(overlays.len(), 1);
        assert_eq!(*overlays[0].occurrence(), marker);
        assert_eq!(overlays[0].bracket().range().visible_range(), range(0, 30));
        assert!((overlays[0].bracket().range().pixels().start_x() - 10.0).abs() < f32::EPSILON);
    }
}
