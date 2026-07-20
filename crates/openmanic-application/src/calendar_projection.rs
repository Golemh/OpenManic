//! Immutable Calendar-day projection contracts for activity, focus, and schedules.
//!
//! A storage adapter resolves the selected local date into its exact UTC midnight-to-midnight
//! range before it constructs a [`CalendarProjectionSource`]. This module deliberately does not
//! calculate time zones: the supplied range can therefore be a normal, 23-hour, or 25-hour day.
//! It clips presentation geometry without mutating canonical source boundaries, and preserves
//! overlaps between the three independently meaningful data kinds.

use core::fmt;

use openmanic_domain::{
    ActivityState, ApplicationId, CategoryId, FocusSessionId, HalfOpenInterval, TrackerRunId,
};

use crate::{
    DataRevision, FocusKind, FocusSnapshot, ProjectionContextKey, ScheduleOccurrence,
    ScheduleOccurrenceId, ScheduleSnapshot, ScheduleTimeError, TimelineRawIntervalId,
    TimelineSourceActivity, project_schedule_occurrences,
};

/// The immutable UTC boundaries of one selected local calendar day.
///
/// The caller resolves local midnight using the configured display-zone and its DST policy.
/// This context only records the resulting half-open UTC range, so it never assumes a day has a
/// fixed duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalendarDayContext {
    context_key: ProjectionContextKey,
    local_day_range: HalfOpenInterval,
}

impl CalendarDayContext {
    /// Creates a day context from an already-resolved local-midnight UTC range.
    #[must_use]
    pub const fn new(context_key: ProjectionContextKey, local_day_range: HalfOpenInterval) -> Self {
        Self {
            context_key,
            local_day_range,
        }
    }

    /// Returns the correlation key for stale-result handling.
    #[must_use]
    pub const fn context_key(self) -> ProjectionContextKey {
        self.context_key
    }

    /// Returns the exact resolved UTC range from one local midnight to the next.
    #[must_use]
    pub const fn local_day_range(self) -> HalfOpenInterval {
        self.local_day_range
    }
}

/// One stable focus source paired with its authoritative canonical interval.
///
/// The focus persistence reader supplies the interval it decoded from durable lifecycle facts.
/// Keeping that read boundary outside this projection avoids treating a paused timer's remaining
/// duration as a fabricated historical end time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarSourceFocus {
    snapshot: FocusSnapshot,
    interval: HalfOpenInterval,
}

impl CalendarSourceFocus {
    /// Pairs a durable focus snapshot with its complete canonical UTC interval.
    #[must_use]
    pub const fn new(snapshot: FocusSnapshot, interval: HalfOpenInterval) -> Self {
        Self { snapshot, interval }
    }

    /// Returns the durable focus snapshot used for source details.
    #[must_use]
    pub const fn snapshot(&self) -> &FocusSnapshot {
        &self.snapshot
    }

    /// Returns the complete canonical interval before Calendar clipping.
    #[must_use]
    pub const fn interval(&self) -> HalfOpenInterval {
        self.interval
    }
}

/// Borrowed, correlated facts for a Calendar projection worker.
#[derive(Clone, Copy, Debug)]
pub struct CalendarProjectionSource<'a> {
    source_revision: DataRevision,
    activities: &'a [TimelineSourceActivity],
    focus_sessions: &'a [CalendarSourceFocus],
    schedules: &'a [ScheduleSnapshot],
}

impl<'a> CalendarProjectionSource<'a> {
    /// Creates a correlated source view without retaining a repository or storage handle.
    #[must_use]
    pub const fn new(
        source_revision: DataRevision,
        activities: &'a [TimelineSourceActivity],
        focus_sessions: &'a [CalendarSourceFocus],
        schedules: &'a [ScheduleSnapshot],
    ) -> Self {
        Self {
            source_revision,
            activities,
            focus_sessions,
            schedules,
        }
    }

    /// Returns the one committed revision shared by every source collection.
    #[must_use]
    pub const fn source_revision(self) -> DataRevision {
        self.source_revision
    }

    /// Returns canonical activity facts with stable storage identities.
    #[must_use]
    pub const fn activities(self) -> &'a [TimelineSourceActivity] {
        self.activities
    }

    /// Returns canonical focus facts with stable focus-session identities.
    #[must_use]
    pub const fn focus_sessions(self) -> &'a [CalendarSourceFocus] {
        self.focus_sessions
    }

    /// Returns schedules which are expanded into shared canonical occurrences.
    #[must_use]
    pub const fn schedules(self) -> &'a [ScheduleSnapshot] {
        self.schedules
    }
}

/// A stable identity for one selectable Calendar block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarBlockId {
    /// A persisted canonical activity record.
    Activity(TimelineRawIntervalId),
    /// A durable focus session.
    Focus(FocusSessionId),
    /// A shared Timeline/Calendar schedule occurrence.
    Schedule(ScheduleOccurrenceId),
}

/// The source-specific details needed by a Calendar renderer or selection inspector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CalendarBlockSource {
    /// Tracked activity with its canonical state and optional application association.
    Activity {
        /// Stable storage identity of the canonical interval.
        raw_id: TimelineRawIntervalId,
        /// Stable tracker run which produced the interval.
        tracker_run_id: TrackerRunId,
        /// Canonical tracking state.
        state: ActivityState,
        /// The resolved application when the state permits one.
        application_id: Option<ApplicationId>,
    },
    /// A durable focus session overlay.
    Focus {
        /// Stable focus-session identity.
        session_id: FocusSessionId,
        /// User-visible focus or short-break kind.
        kind: FocusKind,
        /// Optional user label.
        label: Option<String>,
        /// Optional current category association.
        category_id: Option<CategoryId>,
    },
    /// A shared schedule occurrence overlay.
    Schedule {
        /// Stable occurrence identity reused for editing scopes.
        occurrence_id: ScheduleOccurrenceId,
        /// Current user label.
        label: String,
        /// Optional current category association.
        category_id: Option<CategoryId>,
        /// Whether DST boundary resolution adjusted this occurrence.
        adjusted: bool,
    },
}

/// Precise selection payload used to route an activity block back to Timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityTimelineNavigation {
    raw_id: TimelineRawIntervalId,
    tracker_run_id: TrackerRunId,
    canonical_range: HalfOpenInterval,
    selected_day_range: HalfOpenInterval,
}

impl ActivityTimelineNavigation {
    /// Returns the stable raw activity identity to select in Timeline.
    #[must_use]
    pub const fn raw_id(self) -> TimelineRawIntervalId {
        self.raw_id
    }

    /// Returns the tracker run that produced the selected activity record.
    #[must_use]
    pub const fn tracker_run_id(self) -> TrackerRunId {
        self.tracker_run_id
    }

    /// Returns the un-clipped activity interval for exact Timeline inspection.
    #[must_use]
    pub const fn canonical_range(self) -> HalfOpenInterval {
        self.canonical_range
    }

    /// Returns this record's exact clipped contribution to the selected Calendar day.
    #[must_use]
    pub const fn selected_day_range(self) -> HalfOpenInterval {
        self.selected_day_range
    }
}

/// Continuation markers for a visual block clipped at local midnight.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CalendarContinuation {
    continues_from_previous_day: bool,
    continues_into_next_day: bool,
}

impl CalendarContinuation {
    fn from_ranges(canonical_range: HalfOpenInterval, visual_range: HalfOpenInterval) -> Self {
        Self {
            continues_from_previous_day: canonical_range.start() < visual_range.start(),
            continues_into_next_day: canonical_range.end() > visual_range.end(),
        }
    }

    /// Returns whether the source started before the selected local day.
    #[must_use]
    pub const fn continues_from_previous_day(self) -> bool {
        self.continues_from_previous_day
    }

    /// Returns whether the source ends after the selected local day.
    #[must_use]
    pub const fn continues_into_next_day(self) -> bool {
        self.continues_into_next_day
    }
}

/// One immutable visual block with canonical truth retained for details and navigation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarBlock {
    id: CalendarBlockId,
    source: CalendarBlockSource,
    canonical_range: HalfOpenInterval,
    visual_range: HalfOpenInterval,
    continuation: CalendarContinuation,
}

impl CalendarBlock {
    /// Returns the stable block identity, unchanged when day clipping changes.
    #[must_use]
    pub const fn id(&self) -> CalendarBlockId {
        self.id
    }

    /// Returns source details without requiring a port or a second data model.
    #[must_use]
    pub const fn source(&self) -> &CalendarBlockSource {
        &self.source
    }

    /// Returns complete source truth before visual clipping.
    #[must_use]
    pub const fn canonical_range(&self) -> HalfOpenInterval {
        self.canonical_range
    }

    /// Returns the positive intersection that the vertical day axis renders.
    #[must_use]
    pub const fn visual_range(&self) -> HalfOpenInterval {
        self.visual_range
    }

    /// Returns explicit midnight continuation metadata for the renderer.
    #[must_use]
    pub const fn continuation(&self) -> CalendarContinuation {
        self.continuation
    }

    /// Returns Timeline navigation context only for recorded activity.
    #[must_use]
    pub const fn activity_timeline_navigation(&self) -> Option<ActivityTimelineNavigation> {
        let CalendarBlockSource::Activity {
            raw_id,
            tracker_run_id,
            ..
        } = self.source
        else {
            return None;
        };
        Some(ActivityTimelineNavigation {
            raw_id,
            tracker_run_id,
            canonical_range: self.canonical_range,
            selected_day_range: self.visual_range,
        })
    }
}

/// Immutable Calendar-specific snapshot, intentionally distinct from `TimelineSnapshot`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarDaySnapshot {
    context: CalendarDayContext,
    source_revision: DataRevision,
    activity_blocks: Vec<CalendarBlock>,
    focus_blocks: Vec<CalendarBlock>,
    schedule_blocks: Vec<CalendarBlock>,
}

impl CalendarDaySnapshot {
    /// Returns the Calendar-specific context that supplied the exact local day range.
    #[must_use]
    pub const fn context(&self) -> CalendarDayContext {
        self.context
    }

    /// Returns the one committed source revision shared by all Calendar blocks.
    #[must_use]
    pub const fn source_revision(&self) -> DataRevision {
        self.source_revision
    }

    /// Returns clipped recorded activity blocks. These never overlap each other.
    #[must_use]
    pub fn activity_blocks(&self) -> &[CalendarBlock] {
        &self.activity_blocks
    }

    /// Returns clipped focus overlays. These may overlap activity and schedules.
    #[must_use]
    pub fn focus_blocks(&self) -> &[CalendarBlock] {
        &self.focus_blocks
    }

    /// Returns clipped shared schedule overlays. These may overlap activity and focus.
    #[must_use]
    pub fn schedule_blocks(&self) -> &[CalendarBlock] {
        &self.schedule_blocks
    }
}

/// Pure projector from correlated repository facts to immutable Calendar-day data.
#[derive(Clone, Copy, Debug, Default)]
pub struct CalendarDayProjector;

impl CalendarDayProjector {
    /// Builds Calendar blocks without performing storage, time-zone, or UI work.
    ///
    /// # Errors
    ///
    /// Returns [`CalendarProjectionError`] when canonical activity input overlaps or a schedule
    /// occurrence cannot be resolved. Focus and schedule overlap with other source kinds is
    /// intentionally retained rather than treated as an error.
    pub fn build(
        context: CalendarDayContext,
        source: CalendarProjectionSource<'_>,
    ) -> Result<CalendarDaySnapshot, CalendarProjectionError> {
        let day = context.local_day_range();
        let activity_blocks = project_activities(day, source.activities())?;
        let focus_blocks = project_focus(day, source.focus_sessions());
        let schedule_blocks = project_schedule_occurrences(day, source.schedules())
            .map_err(CalendarProjectionError::ScheduleResolution)?
            .iter()
            .filter_map(|occurrence| schedule_block(day, occurrence))
            .collect();
        Ok(CalendarDaySnapshot {
            context,
            source_revision: source.source_revision(),
            activity_blocks,
            focus_blocks,
            schedule_blocks,
        })
    }
}

fn project_activities(
    day: HalfOpenInterval,
    activities: &[TimelineSourceActivity],
) -> Result<Vec<CalendarBlock>, CalendarProjectionError> {
    let mut sorted = activities.to_vec();
    sorted.sort_by_key(|activity| activity.interval().range().start());
    for pair in sorted.windows(2) {
        let earlier = pair[0];
        let later = pair[1];
        if earlier.interval().range().end() > later.interval().range().start() {
            return Err(CalendarProjectionError::OverlappingActivities {
                earlier: earlier.raw_id(),
                later: later.raw_id(),
            });
        }
    }
    Ok(sorted
        .into_iter()
        .filter_map(|activity| {
            let canonical_range = activity.interval().range();
            let visual_range = clipped_range(canonical_range, day)?;
            Some(CalendarBlock {
                id: CalendarBlockId::Activity(activity.raw_id()),
                source: CalendarBlockSource::Activity {
                    raw_id: activity.raw_id(),
                    tracker_run_id: activity.interval().tracker_run_id(),
                    state: activity.interval().state(),
                    application_id: activity.interval().application_id(),
                },
                canonical_range,
                visual_range,
                continuation: CalendarContinuation::from_ranges(canonical_range, visual_range),
            })
        })
        .collect())
}

fn project_focus(
    day: HalfOpenInterval,
    focus_sessions: &[CalendarSourceFocus],
) -> Vec<CalendarBlock> {
    focus_sessions
        .iter()
        .filter_map(|focus| {
            let canonical_range = focus.interval();
            let visual_range = clipped_range(canonical_range, day)?;
            let snapshot = focus.snapshot();
            Some(CalendarBlock {
                id: CalendarBlockId::Focus(snapshot.session_id()),
                source: CalendarBlockSource::Focus {
                    session_id: snapshot.session_id(),
                    kind: snapshot.kind(),
                    label: snapshot.label().map(str::to_owned),
                    category_id: snapshot.session().category_id(),
                },
                canonical_range,
                visual_range,
                continuation: CalendarContinuation::from_ranges(canonical_range, visual_range),
            })
        })
        .collect()
}

fn schedule_block(day: HalfOpenInterval, occurrence: &ScheduleOccurrence) -> Option<CalendarBlock> {
    let canonical_range = occurrence.interval();
    let visual_range = clipped_range(canonical_range, day)?;
    Some(CalendarBlock {
        id: CalendarBlockId::Schedule(occurrence.id()),
        source: CalendarBlockSource::Schedule {
            occurrence_id: occurrence.id(),
            label: occurrence.label().to_owned(),
            category_id: occurrence.category_id(),
            adjusted: occurrence.adjusted(),
        },
        canonical_range,
        visual_range,
        continuation: CalendarContinuation::from_ranges(canonical_range, visual_range),
    })
}

fn clipped_range(source: HalfOpenInterval, visible: HalfOpenInterval) -> Option<HalfOpenInterval> {
    HalfOpenInterval::try_new(
        source.start().max(visible.start()),
        source.end().min(visible.end()),
    )
    .ok()
}

/// Explains why a Calendar-day snapshot cannot be produced without inventing source truth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarProjectionError {
    /// A persisted recurring schedule boundary could not be resolved.
    ScheduleResolution(ScheduleTimeError),
    /// Canonical activity records contradict their non-overlap invariant.
    OverlappingActivities {
        /// First source record which overlaps a later record.
        earlier: TimelineRawIntervalId,
        /// Later overlapping source record.
        later: TimelineRawIntervalId,
    },
}

impl fmt::Display for CalendarProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScheduleResolution(error) => {
                write!(formatter, "schedule occurrence resolution failed: {error}")
            }
            Self::OverlappingActivities { earlier, later } => write!(
                formatter,
                "calendar activity source {} overlaps activity {}",
                earlier.get(),
                later.get()
            ),
        }
    }
}

impl std::error::Error for CalendarProjectionError {}

#[cfg(test)]
mod tests {
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ActivityInterval, FocusSessionState, HalfOpenInterval,
        OneTimeScheduleId, ScheduleRule, TrackerRunId, UtcMicros,
    };

    use super::{
        CalendarBlockId, CalendarDayContext, CalendarDayProjector, CalendarProjectionSource,
        CalendarSourceFocus,
    };
    use crate::{
        DataRevision, EntityRevision, FocusKind, FocusSnapshot, ProjectionContextKey, ScheduleId,
        ScheduleSnapshot, TimelineRawIntervalId, TimelineSourceActivity,
    };

    #[test]
    fn overnight_activity_keeps_its_stable_identity_and_midnight_continuation() {
        let snapshot = CalendarDayProjector::build(
            context(100, 200),
            CalendarProjectionSource::new(DataRevision::new(4), &[activity(9, 90, 120)], &[], &[]),
        )
        .expect("valid canonical activity projects");

        let block = &snapshot.activity_blocks()[0];
        assert_eq!(
            block.id(),
            CalendarBlockId::Activity(TimelineRawIntervalId::new(9))
        );
        assert_eq!(block.canonical_range(), range(90, 120));
        assert_eq!(block.visual_range(), range(100, 120));
        assert!(block.continuation().continues_from_previous_day());
        assert!(!block.continuation().continues_into_next_day());
    }

    #[test]
    fn overnight_schedule_reuses_its_shared_occurrence_identity_after_clipping() {
        let schedule_id = ScheduleId::OneTime(OneTimeScheduleId::from_bytes([6; 16]));
        let schedule = ScheduleSnapshot::try_new(
            schedule_id,
            ScheduleRule::one_time("Overnight", None, range(90, 120), "Etc/UTC")
                .expect("valid overnight schedule"),
            EntityRevision::new(0),
            UtcMicros::new(1),
        )
        .expect("matching stable schedule ID");
        let snapshot = CalendarDayProjector::build(
            context(100, 200),
            CalendarProjectionSource::new(DataRevision::new(4), &[], &[], &[schedule]),
        )
        .expect("schedule occurrence projects");

        let block = &snapshot.schedule_blocks()[0];
        assert_eq!(
            block.id(),
            CalendarBlockId::Schedule(crate::ScheduleOccurrenceId::OneTime(schedule_id))
        );
        assert_eq!(block.canonical_range(), range(90, 120));
        assert_eq!(block.visual_range(), range(100, 120));
        assert!(block.continuation().continues_from_previous_day());
    }

    #[test]
    fn activity_focus_and_schedule_overlaps_are_retained_as_distinct_blocks() {
        let schedule = ScheduleSnapshot::try_new(
            ScheduleId::OneTime(OneTimeScheduleId::from_bytes([7; 16])),
            ScheduleRule::one_time("Plan", None, range(130, 180), "Etc/UTC")
                .expect("valid one-time schedule"),
            EntityRevision::new(0),
            UtcMicros::new(1),
        )
        .expect("matching stable schedule ID");
        let focus = FocusSnapshot::try_restore(
            openmanic_domain::FocusSessionId::from_bytes([8; 16]),
            FocusKind::Focus,
            Some("Write".to_owned()),
            40,
            None,
            FocusSessionState::Completed {
                started_at: UtcMicros::new(120),
                completed_at: UtcMicros::new(160),
            },
            EntityRevision::new(0),
        )
        .expect("valid completed focus");
        let snapshot = CalendarDayProjector::build(
            context(100, 200),
            CalendarProjectionSource::new(
                DataRevision::new(4),
                &[activity(9, 110, 170)],
                &[CalendarSourceFocus::new(focus, range(120, 160))],
                &[schedule],
            ),
        )
        .expect("overlays are meaningful rather than contradictory");

        assert_eq!(snapshot.activity_blocks().len(), 1);
        assert_eq!(snapshot.focus_blocks().len(), 1);
        assert_eq!(snapshot.schedule_blocks().len(), 1);
        assert_eq!(snapshot.focus_blocks()[0].visual_range(), range(120, 160));
        assert_eq!(
            snapshot.schedule_blocks()[0].visual_range(),
            range(130, 180)
        );
    }

    #[test]
    fn activity_block_exposes_exact_timeline_navigation_context() {
        let snapshot = CalendarDayProjector::build(
            context(100, 200),
            CalendarProjectionSource::new(DataRevision::new(4), &[activity(9, 90, 120)], &[], &[]),
        )
        .expect("valid activity projects");

        let navigation = snapshot.activity_blocks()[0]
            .activity_timeline_navigation()
            .expect("recorded activity is navigable");
        assert_eq!(navigation.raw_id(), TimelineRawIntervalId::new(9));
        assert_eq!(
            navigation.tracker_run_id(),
            TrackerRunId::from_bytes([1; 16])
        );
        assert_eq!(navigation.canonical_range(), range(90, 120));
        assert_eq!(navigation.selected_day_range(), range(100, 120));
    }

    #[test]
    fn supplied_short_and_long_dst_day_ranges_are_preserved_without_time_zone_math() {
        for (start, end) in [(0, 23), (100, 125)] {
            let snapshot = CalendarDayProjector::build(
                context(start, end),
                CalendarProjectionSource::new(
                    DataRevision::new(4),
                    &[activity(9, start - 1, end + 1)],
                    &[],
                    &[],
                ),
            )
            .expect("the resolved day range is authoritative");
            assert_eq!(snapshot.context().local_day_range(), range(start, end));
            assert_eq!(
                snapshot.activity_blocks()[0].visual_range(),
                range(start, end)
            );
            assert!(
                snapshot.activity_blocks()[0]
                    .continuation()
                    .continues_from_previous_day()
            );
            assert!(
                snapshot.activity_blocks()[0]
                    .continuation()
                    .continues_into_next_day()
            );
        }
    }

    fn context(start: i64, end: i64) -> CalendarDayContext {
        CalendarDayContext::new(ProjectionContextKey::new(5), range(start, end))
    }

    fn activity(raw_id: u64, start: i64, end: i64) -> TimelineSourceActivity {
        TimelineSourceActivity::new(
            TimelineRawIntervalId::new(raw_id),
            ActivityInterval::try_new(
                TrackerRunId::from_bytes([1; 16]),
                range(start, end),
                openmanic_domain::ActivityState::Active,
                ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
                    .expect("foreground evidence is valid"),
                Some(openmanic_domain::ApplicationId::from_bytes([2; 16])),
            )
            .expect("active fixture is valid"),
        )
    }

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("fixture range is positive")
    }
}
