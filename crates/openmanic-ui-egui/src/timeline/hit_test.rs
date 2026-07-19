//! Exact half-open interval hit testing through projection indexes.

use openmanic_application::{IntervalIndex, TimelineInterval, TimelineRawFragment};
use openmanic_domain::UtcMicros;

use super::TimelineTransform;

/// A borrowed exact interval selected from a projection band's binary-search index.
#[derive(Clone, Copy, Debug)]
pub struct TimelineHit<'a, T> {
    instant: UtcMicros,
    interval: &'a TimelineInterval<T>,
}

impl<'a, T> TimelineHit<'a, T> {
    /// Returns the exact UTC microsecond selected by the pointer conversion.
    #[must_use]
    pub const fn instant(self) -> UtcMicros {
        self.instant
    }

    /// Returns the immutable presentation interval selected through binary search.
    #[must_use]
    pub const fn interval(self) -> &'a TimelineInterval<T> {
        self.interval
    }

    /// Returns every exact raw record fragment represented by this presentation interval.
    ///
    /// A coalesced band segment can contain more than one source identity. Returning the original
    /// fragments keeps hover and selection detail exact, including for a Powered Off segment.
    #[must_use]
    pub fn raw_fragments(self) -> &'a [TimelineRawFragment] {
        self.interval.raw_fragments()
    }
}

/// Converts one pointer x coordinate once and binary-searches the supplied projection band.
///
/// The projection's [`IntervalIndex::at`] method enforces the half-open boundary rule. In
/// particular, an exact shared boundary belongs to the later interval and the transform's right
/// edge produces no hit.
#[must_use]
pub fn hit_test<T>(
    transform: TimelineTransform,
    index: &IntervalIndex<T>,
    pointer_x: f32,
) -> Option<TimelineHit<'_, T>> {
    let instant = transform.time_at_x(pointer_x)?;
    index
        .at(instant)
        .map(|interval| TimelineHit { instant, interval })
}

#[cfg(test)]
mod tests {
    use openmanic_application::{
        DataRevision, ProjectionContextKey, TimelineContext, TimelineProjectionSource,
        TimelineProjector, TimelineRawIntervalId, TimelineSourceActivity,
    };
    use openmanic_domain::{
        ActivityEvidence, ActivityInterval, ActivityState, HalfOpenInterval,
        PowerTransitionEvidence, TrackerRunId, UtcMicros,
    };

    use super::hit_test;
    use crate::timeline::TimelineTransform;

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("test ranges are positive")
    }

    fn source(raw_id: u64, start: i64, end: i64, state: ActivityState) -> TimelineSourceActivity {
        let evidence = match state {
            ActivityState::PoweredOff => ActivityEvidence::confirmed_shutdown(
                PowerTransitionEvidence::try_new(UtcMicros::new(start), UtcMicros::new(end))
                    .expect("the test power span is positive"),
            ),
            _ => ActivityEvidence::try_from_cause(openmanic_domain::ActivityCause::IdleThreshold)
                .expect("ordinary test evidence is valid"),
        };
        let activity = ActivityInterval::try_new(
            TrackerRunId::from_bytes([7; 16]),
            range(start, end),
            state,
            evidence,
            None,
        )
        .expect("the test interval satisfies state invariants");
        TimelineSourceActivity::new(TimelineRawIntervalId::new(raw_id), activity)
    }

    #[test]
    fn binary_search_preserves_boundary_identity_and_powered_off_fragments() {
        let activities = [
            source(10, 0, 10, ActivityState::Idle),
            source(11, 10, 20, ActivityState::PoweredOff),
            source(12, 20, 30, ActivityState::Unavailable),
        ];
        let snapshot = TimelineProjector::build(
            TimelineContext::new(ProjectionContextKey::new(1), range(0, 30)),
            TimelineProjectionSource::new(DataRevision::new(1), &activities, &[]),
        )
        .expect("the ordered fixture produces a timeline snapshot");
        let transform =
            TimelineTransform::try_new(range(0, 30), 0.0, 300.0).expect("positive finite geometry");

        let left = hit_test(transform, snapshot.activity_band(), 99.9).expect("left hit");
        assert_eq!(left.instant(), UtcMicros::new(9));
        assert_eq!(
            left.raw_fragments()[0].raw_id(),
            TimelineRawIntervalId::new(10)
        );

        let powered_off = hit_test(transform, snapshot.activity_band(), 100.0)
            .expect("boundary belongs to the powered-off segment");
        assert_eq!(powered_off.instant(), UtcMicros::new(10));
        assert_eq!(
            powered_off.raw_fragments()[0].raw_id(),
            TimelineRawIntervalId::new(11)
        );
        assert_eq!(
            powered_off.interval().value().state(),
            ActivityState::PoweredOff
        );

        let right = hit_test(transform, snapshot.activity_band(), 200.0).expect("right hit");
        assert_eq!(
            right.raw_fragments()[0].raw_id(),
            TimelineRawIntervalId::new(12)
        );
        assert!(hit_test(transform, snapshot.activity_band(), 300.0).is_none());
    }
}
