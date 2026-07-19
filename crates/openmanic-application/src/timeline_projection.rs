//! Timeline-specific immutable read projection and binary-search indexes.
//!
//! The projector is deliberately a pure application-layer calculation. A storage adapter opens a
//! short correlated read transaction, maps only the requested visible facts into
//! [`TimelineProjectionSource`], and invokes [`TimelineProjector`] on a worker. The UI receives
//! only the resulting [`SnapshotEnvelope`] and never needs a repository or a history clone.

use core::fmt;
use std::{collections::HashMap, sync::Arc};

use openmanic_domain::{
    ActivityInterval, ActivityState, ApplicationId, CategoryId, HalfOpenInterval, TrackerRunId,
    UtcMicros,
};

use crate::{
    DataRevision, ProjectionContextKey, ProjectionRequest, SchemaRevision, SnapshotEnvelope,
};

/// The schema revision of a [`TimelineSnapshot`] published to a renderer.
pub const TIMELINE_SNAPSHOT_SCHEMA_REVISION: SchemaRevision = SchemaRevision::new(1);

/// The normalized, immutable context for one timeline query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineContext {
    context_key: ProjectionContextKey,
    visible_range: HalfOpenInterval,
}

impl TimelineContext {
    /// Creates a context with the exact range represented by the timeline transform.
    #[must_use]
    pub const fn new(context_key: ProjectionContextKey, visible_range: HalfOpenInterval) -> Self {
        Self {
            context_key,
            visible_range,
        }
    }

    /// Returns the normalized context key used for stale-result rejection.
    #[must_use]
    pub const fn context_key(self) -> ProjectionContextKey {
        self.context_key
    }

    /// Returns the half-open UTC range for which this snapshot is gap-free.
    #[must_use]
    pub const fn visible_range(self) -> HalfOpenInterval {
        self.visible_range
    }
}

/// Stable storage identity of a raw activity record.
///
/// The storage adapter supplies this value from its stable row identity. It is kept separate
/// from presentation interval boundaries so inspection and hit testing can always recover the
/// exact record(s) that produced a coalesced segment.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimelineRawIntervalId(u64);

impl TimelineRawIntervalId {
    /// Creates an identity from the storage adapter's stable row value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact adapter-provided identity value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One canonical activity record supplied by a storage read adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineSourceActivity {
    raw_id: TimelineRawIntervalId,
    interval: ActivityInterval,
}

impl TimelineSourceActivity {
    /// Pairs a stable raw identity with its canonical interval.
    #[must_use]
    pub const fn new(raw_id: TimelineRawIntervalId, interval: ActivityInterval) -> Self {
        Self { raw_id, interval }
    }

    /// Returns the stable source record identity.
    #[must_use]
    pub const fn raw_id(self) -> TimelineRawIntervalId {
        self.raw_id
    }

    /// Returns the canonical, un-clipped activity interval.
    #[must_use]
    pub const fn interval(self) -> ActivityInterval {
        self.interval
    }
}

/// Current category association for one known application in a correlated read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineApplication {
    application_id: ApplicationId,
    category_id: Option<CategoryId>,
}

impl TimelineApplication {
    /// Creates one current application-to-category association.
    #[must_use]
    pub const fn new(application_id: ApplicationId, category_id: Option<CategoryId>) -> Self {
        Self {
            application_id,
            category_id,
        }
    }

    /// Returns the application ID.
    #[must_use]
    pub const fn application_id(self) -> ApplicationId {
        self.application_id
    }

    /// Returns the current category, or `None` when the application is uncategorized.
    #[must_use]
    pub const fn category_id(self) -> Option<CategoryId> {
        self.category_id
    }
}

/// Borrowed, correlated raw facts already selected by a background storage query.
///
/// Callers must pass only records which may intersect the requested visible range. The projector
/// still clips and rejects non-intersecting input defensively, so its immutable output has a
/// bounded range and no reference to a repository or full-history vector.
#[derive(Clone, Copy, Debug)]
pub struct TimelineProjectionSource<'a> {
    source_revision: DataRevision,
    activities: &'a [TimelineSourceActivity],
    applications: &'a [TimelineApplication],
}

impl<'a> TimelineProjectionSource<'a> {
    /// Creates a source view read atomically at `source_revision`.
    #[must_use]
    pub const fn new(
        source_revision: DataRevision,
        activities: &'a [TimelineSourceActivity],
        applications: &'a [TimelineApplication],
    ) -> Self {
        Self {
            source_revision,
            activities,
            applications,
        }
    }

    /// Returns the one committed revision shared by all source facts.
    #[must_use]
    pub const fn source_revision(self) -> DataRevision {
        self.source_revision
    }

    /// Returns the adapter-selected activity records.
    #[must_use]
    pub const fn activities(self) -> &'a [TimelineSourceActivity] {
        self.activities
    }

    /// Returns the correlated current application associations.
    #[must_use]
    pub const fn applications(self) -> &'a [TimelineApplication] {
        self.applications
    }
}

/// Presentation value for the timeline's category band.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CategoryBandValue {
    /// A known current category.
    Category(CategoryId),
    /// A known application without a category assignment, or unresolved application metadata.
    Uncategorized,
    /// A state that cannot truthfully be represented as application/category usage.
    NonApplicationState(ActivityState),
}

/// Presentation value for the timeline's activity band.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityStateValue {
    /// A resolved application was active.
    Active,
    /// The idle threshold was met.
    Idle,
    /// Tracking was paused explicitly by the user.
    PausedByUser,
    /// The current application was excluded by policy.
    Excluded,
    /// The platform could not supply usable evidence.
    Unavailable,
    /// Qualifying shutdown/startup evidence bounds the interval.
    PoweredOff,
    /// Evidence is missing or contradictory.
    UnknownMissing,
}

impl ActivityStateValue {
    /// Returns the canonical state represented by this band value.
    #[must_use]
    pub const fn state(self) -> ActivityState {
        match self {
            Self::Active => ActivityState::Active,
            Self::Idle => ActivityState::Idle,
            Self::PausedByUser => ActivityState::PausedByUser,
            Self::Excluded => ActivityState::Excluded,
            Self::Unavailable => ActivityState::Unavailable,
            Self::PoweredOff => ActivityState::PoweredOff,
            Self::UnknownMissing => ActivityState::UnknownMissing,
        }
    }

    const fn from_state(state: ActivityState) -> Self {
        match state {
            ActivityState::Active => Self::Active,
            ActivityState::Idle => Self::Idle,
            ActivityState::PausedByUser => Self::PausedByUser,
            ActivityState::Excluded => Self::Excluded,
            ActivityState::Unavailable => Self::Unavailable,
            ActivityState::PoweredOff => Self::PoweredOff,
            ActivityState::UnknownMissing => Self::UnknownMissing,
        }
    }
}

/// Presentation value for the timeline's application band.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationBandValue {
    /// A known application record.
    Application(ApplicationId),
    /// A state that deliberately has no application association.
    NoApplication(ActivityState),
    /// The canonical interval referenced an application absent from the correlated catalog.
    UnresolvedApplication,
}

/// The exact clipped contribution of one source record to a presentation interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineRawFragment {
    raw_id: TimelineRawIntervalId,
    tracker_run_id: TrackerRunId,
    raw_range: HalfOpenInterval,
    visible_range: HalfOpenInterval,
}

impl TimelineRawFragment {
    fn from_source(source: TimelineSourceActivity, visible_range: HalfOpenInterval) -> Self {
        Self {
            raw_id: source.raw_id(),
            tracker_run_id: source.interval().tracker_run_id(),
            raw_range: source.interval().range(),
            visible_range,
        }
    }

    /// Returns the exact stable source record identity.
    #[must_use]
    pub const fn raw_id(self) -> TimelineRawIntervalId {
        self.raw_id
    }

    /// Returns the tracker run which produced the source record.
    #[must_use]
    pub const fn tracker_run_id(self) -> TrackerRunId {
        self.tracker_run_id
    }

    /// Returns the original, un-clipped canonical boundary.
    #[must_use]
    pub const fn raw_range(self) -> HalfOpenInterval {
        self.raw_range
    }

    /// Returns this raw record's exact contribution inside the visible range.
    #[must_use]
    pub const fn visible_range(self) -> HalfOpenInterval {
        self.visible_range
    }
}

/// One immutable, half-open presentation interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineInterval<T> {
    range: HalfOpenInterval,
    value: T,
    raw_fragments: Arc<[TimelineRawFragment]>,
    synthesized_unknown_ranges: Arc<[HalfOpenInterval]>,
}

impl<T> TimelineInterval<T> {
    /// Returns the displayed half-open range.
    #[must_use]
    pub const fn range(&self) -> HalfOpenInterval {
        self.range
    }

    /// Returns the presentation value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns exact source records and clipped boundaries represented by this interval.
    #[must_use]
    pub fn raw_fragments(&self) -> &[TimelineRawFragment] {
        self.raw_fragments.as_ref()
    }

    /// Returns uncovered ranges explicitly synthesized as `UnknownMissing`.
    #[must_use]
    pub fn synthesized_unknown_ranges(&self) -> &[HalfOpenInterval] {
        self.synthesized_unknown_ranges.as_ref()
    }
}

/// Immutable, gap-free intervals searched with binary search by UTC instant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntervalIndex<T> {
    visible_range: HalfOpenInterval,
    intervals: Arc<[TimelineInterval<T>]>,
}

impl<T> IntervalIndex<T> {
    fn new(visible_range: HalfOpenInterval, intervals: Vec<TimelineInterval<T>>) -> Self {
        Self {
            visible_range,
            intervals: Arc::from(intervals),
        }
    }

    /// Returns the range for which this index is complete and gap-free.
    #[must_use]
    pub const fn visible_range(&self) -> HalfOpenInterval {
        self.visible_range
    }

    /// Returns the immutable raw presentation intervals.
    #[must_use]
    pub fn intervals(&self) -> &[TimelineInterval<T>] {
        self.intervals.as_ref()
    }

    /// Finds the interval containing `instant` using the half-open boundary rule.
    #[must_use]
    pub fn at(&self, instant: UtcMicros) -> Option<&TimelineInterval<T>> {
        if instant < self.visible_range.start() || instant >= self.visible_range.end() {
            return None;
        }
        let index = self
            .intervals
            .partition_point(|interval| interval.range.end() <= instant);
        self.intervals
            .get(index)
            .filter(|interval| interval.range.start() <= instant && instant < interval.range.end())
    }

    /// Returns the smallest contiguous slice intersecting `range`.
    #[must_use]
    pub fn intersecting(&self, range: HalfOpenInterval) -> &[TimelineInterval<T>] {
        let first = self
            .intervals
            .partition_point(|interval| interval.range.end() <= range.start());
        let end = self
            .intervals
            .partition_point(|interval| interval.range.start() < range.end());
        if first >= end {
            &[]
        } else {
            &self.intervals[first..end]
        }
    }
}

/// Aggregates retained independently of paint-time interval aggregation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineTotals {
    visible: u64,
    known: u64,
    unknown_missing: u64,
}

impl TimelineTotals {
    fn new(visible_range: HalfOpenInterval, unknown_missing_duration_us: u64) -> Self {
        let visible = visible_range.duration_us();
        Self {
            visible,
            known: visible.saturating_sub(unknown_missing_duration_us),
            unknown_missing: unknown_missing_duration_us,
        }
    }

    /// Returns the duration of the visible range.
    #[must_use]
    pub const fn visible_duration_us(self) -> u64 {
        self.visible
    }

    /// Returns the duration backed by non-unknown activity evidence.
    #[must_use]
    pub const fn known_duration_us(self) -> u64 {
        self.known
    }

    /// Returns the duration represented as `UnknownMissing`.
    #[must_use]
    pub const fn unknown_missing_duration_us(self) -> u64 {
        self.unknown_missing
    }
}

/// Indicates whether every visible instant had non-unknown evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataCompleteness {
    /// No visible instant was represented as `UnknownMissing`.
    Complete,
    /// One or more visible instants are explicitly unknown rather than inferred.
    Partial {
        /// Total duration represented as `UnknownMissing`.
        unknown_missing_duration_us: u64,
    },
}

/// Immutable, correlated timeline bands ready for an action-only renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineSnapshot {
    context: TimelineContext,
    source_revision: DataRevision,
    category_band: IntervalIndex<CategoryBandValue>,
    activity_band: IntervalIndex<ActivityStateValue>,
    application_band: IntervalIndex<ApplicationBandValue>,
    totals: TimelineTotals,
    completeness: DataCompleteness,
}

impl TimelineSnapshot {
    /// Returns the normalized context which produced this snapshot.
    #[must_use]
    pub const fn context(&self) -> TimelineContext {
        self.context
    }

    /// Returns the one committed source revision shared by every band and total.
    #[must_use]
    pub const fn source_revision(&self) -> DataRevision {
        self.source_revision
    }

    /// Returns the requested, gap-free timeline range.
    #[must_use]
    pub const fn visible_range(&self) -> HalfOpenInterval {
        self.context.visible_range()
    }

    /// Returns the independently normalized category-band index.
    #[must_use]
    pub const fn category_band(&self) -> &IntervalIndex<CategoryBandValue> {
        &self.category_band
    }

    /// Returns the independently normalized activity-band index.
    #[must_use]
    pub const fn activity_band(&self) -> &IntervalIndex<ActivityStateValue> {
        &self.activity_band
    }

    /// Returns the independently normalized application-band index.
    #[must_use]
    pub const fn application_band(&self) -> &IntervalIndex<ApplicationBandValue> {
        &self.application_band
    }

    /// Returns exact raw totals that paint-time aggregation must preserve.
    #[must_use]
    pub const fn totals(&self) -> TimelineTotals {
        self.totals
    }

    /// Returns whether visible instants were fully attributed.
    #[must_use]
    pub const fn completeness(&self) -> DataCompleteness {
        self.completeness
    }
}

/// Pure background projector from correlated repository facts to immutable renderer data.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimelineProjector;

impl TimelineProjector {
    /// Projects a correlated source into an immutable timeline snapshot envelope.
    ///
    /// The returned envelope uses the request's correlation fields, allowing existing
    /// [`crate::ProjectionSlotState`] stale-result rejection to protect the UI even if a
    /// cancelled worker completes late.
    ///
    /// # Errors
    ///
    /// Returns [`TimelineProjectionError`] when repository facts are contradictory or cannot
    /// produce one gap-free interpretation without guessing.
    pub fn project(
        request: &ProjectionRequest<TimelineContext>,
        source: TimelineProjectionSource<'_>,
    ) -> Result<SnapshotEnvelope<TimelineSnapshot>, TimelineProjectionError> {
        let context = *request.payload();
        if context.context_key() != request.context_key() {
            return Err(TimelineProjectionError::ContextKeyMismatch {
                request: request.context_key(),
                context: context.context_key(),
            });
        }
        let snapshot = Self::build(context, source)?;
        Ok(SnapshotEnvelope::new(
            request.request_id(),
            request.slot(),
            request.context_key(),
            source.source_revision(),
            TIMELINE_SNAPSHOT_SCHEMA_REVISION,
            snapshot,
        ))
    }

    /// Builds a timeline snapshot from one correlated, background-read source.
    ///
    /// This function deliberately accepts borrowed source slices and creates new `Arc` arrays
    /// only for clipped visible intervals. It neither opens storage nor retains the source view.
    ///
    /// # Errors
    ///
    /// Returns [`TimelineProjectionError`] when raw intervals overlap or application facts are
    /// duplicated.
    pub fn build(
        context: TimelineContext,
        source: TimelineProjectionSource<'_>,
    ) -> Result<TimelineSnapshot, TimelineProjectionError> {
        let applications = application_categories(source.applications())?;
        let visible_range = context.visible_range();
        let raw_segments = raw_segments(visible_range, source.activities())?;

        let unknown_missing_duration_us = raw_segments
            .iter()
            .filter(|segment| segment.state() == ActivityState::UnknownMissing)
            .map(|segment| segment.range().duration_us())
            .sum();
        let totals = TimelineTotals::new(visible_range, unknown_missing_duration_us);
        let completeness = if unknown_missing_duration_us == 0 {
            DataCompleteness::Complete
        } else {
            DataCompleteness::Partial {
                unknown_missing_duration_us,
            }
        };

        let category_band = IntervalIndex::new(
            visible_range,
            coalesce(raw_segments.iter().map(|segment| {
                PreparedInterval::from_raw(segment, category_value(segment, &applications))
            }))?,
        );
        let activity_band = IntervalIndex::new(
            visible_range,
            coalesce(raw_segments.iter().map(|segment| {
                PreparedInterval::from_raw(segment, ActivityStateValue::from_state(segment.state()))
            }))?,
        );
        let application_band = IntervalIndex::new(
            visible_range,
            coalesce(raw_segments.iter().map(|segment| {
                PreparedInterval::from_raw(segment, application_value(segment, &applications))
            }))?,
        );

        Ok(TimelineSnapshot {
            context,
            source_revision: source.source_revision(),
            category_band,
            activity_band,
            application_band,
            totals,
            completeness,
        })
    }
}

fn raw_segments(
    visible_range: HalfOpenInterval,
    source_activities: &[TimelineSourceActivity],
) -> Result<Vec<RawSegment>, TimelineProjectionError> {
    let mut activities: Vec<_> = source_activities
        .iter()
        .copied()
        .filter(|activity| activity.interval().range().overlaps(visible_range))
        .collect();
    activities.sort_by_key(|activity| {
        (
            activity.interval().range().start(),
            activity.interval().range().end(),
            activity.raw_id(),
        )
    });

    let mut segments: Vec<RawSegment> = Vec::with_capacity(activities.len().saturating_add(1));
    let mut cursor = visible_range.start();
    for activity in activities {
        let Some(clipped) = clipped_range(activity.interval().range(), visible_range) else {
            continue;
        };
        if clipped.start() < cursor {
            return Err(TimelineProjectionError::OverlappingActivities {
                earlier: segments
                    .last()
                    .and_then(|segment| segment.raw_id())
                    .unwrap_or(activity.raw_id()),
                later: activity.raw_id(),
            });
        }
        if cursor < clipped.start() {
            segments.push(RawSegment::unknown(positive_range(
                cursor,
                clipped.start(),
            )?));
        }
        segments.push(RawSegment::activity(activity, clipped));
        cursor = clipped.end();
    }
    if cursor < visible_range.end() {
        segments.push(RawSegment::unknown(positive_range(
            cursor,
            visible_range.end(),
        )?));
    }
    Ok(segments)
}

#[derive(Clone, Copy, Debug)]
enum RawSegment {
    Activity {
        source: TimelineSourceActivity,
        clipped_range: HalfOpenInterval,
    },
    Unknown {
        range: HalfOpenInterval,
    },
}

impl RawSegment {
    fn activity(source: TimelineSourceActivity, clipped_range: HalfOpenInterval) -> Self {
        Self::Activity {
            source,
            clipped_range,
        }
    }

    fn unknown(range: HalfOpenInterval) -> Self {
        Self::Unknown { range }
    }

    const fn range(self) -> HalfOpenInterval {
        match self {
            Self::Activity { clipped_range, .. } => clipped_range,
            Self::Unknown { range } => range,
        }
    }

    const fn state(self) -> ActivityState {
        match self {
            Self::Activity { source, .. } => source.interval().state(),
            Self::Unknown { .. } => ActivityState::UnknownMissing,
        }
    }

    const fn application_id(self) -> Option<ApplicationId> {
        match self {
            Self::Activity { source, .. } => source.interval().application_id(),
            Self::Unknown { .. } => None,
        }
    }

    const fn raw_id(self) -> Option<TimelineRawIntervalId> {
        match self {
            Self::Activity { source, .. } => Some(source.raw_id()),
            Self::Unknown { .. } => None,
        }
    }

    fn raw_fragment(self) -> Option<TimelineRawFragment> {
        match self {
            Self::Activity {
                source,
                clipped_range,
            } => Some(TimelineRawFragment::from_source(source, clipped_range)),
            Self::Unknown { .. } => None,
        }
    }

    const fn is_synthesized_unknown(self) -> bool {
        matches!(self, Self::Unknown { .. })
    }
}

struct PreparedInterval<T> {
    range: HalfOpenInterval,
    value: T,
    raw_fragments: Vec<TimelineRawFragment>,
    synthesized_unknown_ranges: Vec<HalfOpenInterval>,
}

impl<T> PreparedInterval<T> {
    fn from_raw(segment: &RawSegment, value: T) -> Self {
        let mut raw_fragments = Vec::new();
        let mut synthesized_unknown_ranges = Vec::new();
        if let Some(fragment) = segment.raw_fragment() {
            raw_fragments.push(fragment);
        }
        if segment.is_synthesized_unknown() {
            synthesized_unknown_ranges.push(segment.range());
        }
        Self {
            range: segment.range(),
            value,
            raw_fragments,
            synthesized_unknown_ranges,
        }
    }

    fn into_timeline(self) -> TimelineInterval<T> {
        TimelineInterval {
            range: self.range,
            value: self.value,
            raw_fragments: Arc::from(self.raw_fragments),
            synthesized_unknown_ranges: Arc::from(self.synthesized_unknown_ranges),
        }
    }
}

fn coalesce<T: Eq>(
    intervals: impl Iterator<Item = PreparedInterval<T>>,
) -> Result<Vec<TimelineInterval<T>>, TimelineProjectionError> {
    let mut coalesced = Vec::new();
    for interval in intervals {
        let Some(previous) = coalesced.last_mut() else {
            coalesced.push(interval);
            continue;
        };
        if previous.value == interval.value && previous.range.end() == interval.range.start() {
            previous.range = positive_range(previous.range.start(), interval.range.end())?;
            previous.raw_fragments.extend(interval.raw_fragments);
            previous
                .synthesized_unknown_ranges
                .extend(interval.synthesized_unknown_ranges);
        } else {
            coalesced.push(interval);
        }
    }
    Ok(coalesced
        .into_iter()
        .map(PreparedInterval::into_timeline)
        .collect())
}

fn category_value(
    segment: &RawSegment,
    applications: &HashMap<ApplicationId, Option<CategoryId>>,
) -> CategoryBandValue {
    let state = segment.state();
    if state != ActivityState::Active {
        return CategoryBandValue::NonApplicationState(state);
    }
    match segment
        .application_id()
        .and_then(|id| applications.get(&id))
    {
        Some(Some(category)) => CategoryBandValue::Category(*category),
        Some(None) | None => CategoryBandValue::Uncategorized,
    }
}

fn application_value(
    segment: &RawSegment,
    applications: &HashMap<ApplicationId, Option<CategoryId>>,
) -> ApplicationBandValue {
    let state = segment.state();
    if state != ActivityState::Active {
        return ApplicationBandValue::NoApplication(state);
    }
    match segment.application_id() {
        Some(application_id) if applications.contains_key(&application_id) => {
            ApplicationBandValue::Application(application_id)
        }
        Some(_) | None => ApplicationBandValue::UnresolvedApplication,
    }
}

fn application_categories(
    applications: &[TimelineApplication],
) -> Result<HashMap<ApplicationId, Option<CategoryId>>, TimelineProjectionError> {
    let mut categories = HashMap::with_capacity(applications.len());
    for application in applications {
        if categories
            .insert(application.application_id(), application.category_id())
            .is_some()
        {
            return Err(TimelineProjectionError::DuplicateApplication {
                application_id: application.application_id(),
            });
        }
    }
    Ok(categories)
}

fn clipped_range(source: HalfOpenInterval, visible: HalfOpenInterval) -> Option<HalfOpenInterval> {
    let start = source.start().max(visible.start());
    let end = source.end().min(visible.end());
    HalfOpenInterval::try_new(start, end).ok()
}

fn positive_range(
    start: UtcMicros,
    end: UtcMicros,
) -> Result<HalfOpenInterval, TimelineProjectionError> {
    HalfOpenInterval::try_new(start, end)
        .map_err(|_| TimelineProjectionError::InvalidRange { start, end })
}

/// Explains why a source cannot be projected without inventing an interpretation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineProjectionError {
    /// Internal source boundaries could not form a positive half-open interval.
    InvalidRange {
        /// Requested inclusive start boundary.
        start: UtcMicros,
        /// Requested exclusive end boundary.
        end: UtcMicros,
    },
    /// The request and its typed timeline context used incompatible stale-result keys.
    ContextKeyMismatch {
        /// Key carried by the generic projection request.
        request: ProjectionContextKey,
        /// Key embedded in the typed timeline context.
        context: ProjectionContextKey,
    },
    /// The source contains overlapping canonical records in the same visible query.
    OverlappingActivities {
        /// First source record which overlaps a later record.
        earlier: TimelineRawIntervalId,
        /// Later overlapping source record.
        later: TimelineRawIntervalId,
    },
    /// The correlated application view duplicated one stable application ID.
    DuplicateApplication {
        /// Duplicated application identity.
        application_id: ApplicationId,
    },
}

impl fmt::Display for TimelineProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange { start, end } => write!(
                formatter,
                "timeline interval [{}, {}) must be positive",
                start.get(),
                end.get()
            ),
            Self::ContextKeyMismatch { request, context } => write!(
                formatter,
                "timeline context key {} does not match request key {}",
                context.get(),
                request.get()
            ),
            Self::OverlappingActivities { earlier, later } => write!(
                formatter,
                "timeline source activity {} overlaps activity {}",
                earlier.get(),
                later.get()
            ),
            Self::DuplicateApplication { application_id } => {
                write!(
                    formatter,
                    "timeline source contains duplicate application {application_id:?}"
                )
            }
        }
    }
}

impl std::error::Error for TimelineProjectionError {}

#[cfg(test)]
mod tests {
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ApplicationId, CategoryId, HalfOpenInterval, TrackerRunId,
    };

    use super::{
        ActivityStateValue, ApplicationBandValue, CategoryBandValue, DataCompleteness,
        TimelineApplication, TimelineContext, TimelineProjectionError, TimelineProjectionSource,
        TimelineProjector, TimelineRawIntervalId, TimelineSnapshot, TimelineSourceActivity,
    };
    use crate::{
        DataRevision, ProjectionContextKey, ProjectionRequest, ProjectionSlot, ProjectionSlotState,
        RequestId, SnapshotRejection,
    };
    use openmanic_domain::{ActivityInterval, ActivityState, UtcMicros};

    #[test]
    fn projection_preserves_raw_identity_and_synthesizes_explicit_unknown_gaps() {
        let snapshot = presentation_snapshot();
        let category = snapshot.category_band().intervals();
        assert_eq!(category.len(), 4);
        assert_eq!(
            *category[0].value(),
            CategoryBandValue::Category(category_id(7))
        );
        assert_eq!(
            *category[1].value(),
            CategoryBandValue::NonApplicationState(ActivityState::Idle)
        );
        assert_eq!(*category[2].value(), CategoryBandValue::Uncategorized);
        assert_eq!(
            *category[3].value(),
            CategoryBandValue::NonApplicationState(ActivityState::UnknownMissing)
        );
        assert_eq!(
            category[0].raw_fragments()[0].raw_id(),
            TimelineRawIntervalId::new(1)
        );
        assert_eq!(category[0].raw_fragments()[0].raw_range(), range(0, 10));
        assert_eq!(category[0].raw_fragments()[0].visible_range(), range(0, 10));
        assert!(category[3].raw_fragments().is_empty());
        assert_eq!(category[3].synthesized_unknown_ranges(), &[range(30, 40)]);
        assert_eq!(
            *snapshot
                .activity_band()
                .at(UtcMicros::new(30))
                .expect("gap is indexed")
                .value(),
            ActivityStateValue::UnknownMissing
        );
        assert!(snapshot.activity_band().at(UtcMicros::new(40)).is_none());
        assert_eq!(snapshot.totals().visible_duration_us(), 40);
        assert_eq!(snapshot.totals().known_duration_us(), 30);
        assert_eq!(snapshot.totals().unknown_missing_duration_us(), 10);
        assert_eq!(
            snapshot.completeness(),
            DataCompleteness::Partial {
                unknown_missing_duration_us: 10
            }
        );
    }

    #[test]
    fn application_band_exposes_known_unresolved_and_no_application_values() {
        let snapshot = presentation_snapshot();
        let application = snapshot.application_band().intervals();
        assert_eq!(
            *application[0].value(),
            ApplicationBandValue::Application(application_id(1))
        );
        assert_eq!(
            *application[1].value(),
            ApplicationBandValue::NoApplication(ActivityState::Idle)
        );
        assert_eq!(
            *application[2].value(),
            ApplicationBandValue::UnresolvedApplication
        );
        assert_eq!(
            *application[3].value(),
            ApplicationBandValue::NoApplication(ActivityState::UnknownMissing)
        );
    }

    #[test]
    fn independently_coalesced_bands_retain_every_raw_fragment_and_clip_to_visible_range() {
        let activities = [
            source_activity(1, active(1, 0, 15, 1)),
            source_activity(2, active(1, 15, 30, 1)),
            source_activity(3, state(1, 30, 50, ActivityState::PausedByUser)),
        ];
        let applications = [TimelineApplication::new(
            application_id(1),
            Some(category_id(3)),
        )];
        let snapshot = TimelineProjector::build(
            context(10, 40),
            TimelineProjectionSource::new(DataRevision::new(9), &activities, &applications),
        )
        .expect("adjacent canonical intervals are valid");

        let categories = snapshot.category_band().intervals();
        assert_eq!(categories.len(), 2);
        assert_eq!(categories[0].range(), range(10, 30));
        assert_eq!(categories[0].raw_fragments().len(), 2);
        assert_eq!(
            categories[0].raw_fragments()[0].raw_id(),
            TimelineRawIntervalId::new(1)
        );
        assert_eq!(
            categories[0].raw_fragments()[0].visible_range(),
            range(10, 15)
        );
        assert_eq!(
            categories[0].raw_fragments()[1].raw_id(),
            TimelineRawIntervalId::new(2)
        );
        assert_eq!(categories[1].range(), range(30, 40));
        assert_eq!(
            snapshot.activity_band().intersecting(range(14, 31)).len(),
            2
        );
        assert_eq!(snapshot.visible_range(), range(10, 40));
    }

    #[test]
    fn contradictory_overlaps_and_duplicate_catalog_facts_are_rejected() {
        let activities = [
            source_activity(1, active(1, 0, 20, 1)),
            source_activity(2, active(1, 10, 30, 1)),
        ];
        assert_eq!(
            TimelineProjector::build(
                context(0, 30),
                TimelineProjectionSource::new(DataRevision::new(1), &activities, &[]),
            ),
            Err(TimelineProjectionError::OverlappingActivities {
                earlier: TimelineRawIntervalId::new(1),
                later: TimelineRawIntervalId::new(2),
            })
        );

        let applications = [
            TimelineApplication::new(application_id(1), None),
            TimelineApplication::new(application_id(1), Some(category_id(2))),
        ];
        assert_eq!(
            TimelineProjector::build(
                context(0, 30),
                TimelineProjectionSource::new(DataRevision::new(1), &[], &applications),
            ),
            Err(TimelineProjectionError::DuplicateApplication {
                application_id: application_id(1),
            })
        );
    }

    #[test]
    fn late_timeline_snapshot_is_rejected_by_the_frozen_correlation_contract() {
        let initial_request = request(11, 3, 5, 10, context(0, 10));
        let source = TimelineProjectionSource::new(DataRevision::new(9), &[], &[]);
        let snapshot = TimelineProjector::project(&initial_request, source)
            .expect("projection itself may be computed from a stale source");
        let mut receiver = ProjectionSlotState::new(&initial_request);
        assert_eq!(
            receiver.accept_if_current(&snapshot),
            Err(SnapshotRejection::SourceRevisionOlderThanRequired {
                source: DataRevision::new(9),
                required: DataRevision::new(10),
            })
        );

        let current = request(12, 3, 6, 10, context(0, 10));
        receiver
            .replace_current_request(&current)
            .expect("slot stays stable while a new request supersedes the old one");
        assert_eq!(
            receiver.accept_if_current(&snapshot),
            Err(SnapshotRejection::RequestNotCurrent {
                expected: RequestId::new(12),
                received: RequestId::new(11),
            })
        );
    }

    fn request(
        request_id: u64,
        slot: u64,
        context_key: u64,
        revision: u64,
        context: TimelineContext,
    ) -> ProjectionRequest<TimelineContext> {
        ProjectionRequest::new(
            RequestId::new(request_id),
            ProjectionSlot::new(slot),
            ProjectionContextKey::new(context_key),
            DataRevision::new(revision),
            context,
        )
    }

    fn presentation_snapshot() -> TimelineSnapshot {
        let activities = [
            source_activity(1, active(1, 0, 10, 1)),
            source_activity(2, state(1, 10, 20, ActivityState::Idle)),
            source_activity(3, active(1, 20, 30, 2)),
        ];
        let applications = [TimelineApplication::new(
            application_id(1),
            Some(category_id(7)),
        )];
        TimelineProjector::build(
            context(0, 40),
            TimelineProjectionSource::new(DataRevision::new(8), &activities, &applications),
        )
        .expect("fixture is a gap-free sequence before its final explicit gap")
    }

    fn context(start: i64, end: i64) -> TimelineContext {
        TimelineContext::new(ProjectionContextKey::new(5), range(start, end))
    }

    fn source_activity(raw_id: u64, interval: ActivityInterval) -> TimelineSourceActivity {
        TimelineSourceActivity::new(TimelineRawIntervalId::new(raw_id), interval)
    }

    fn active(run: u8, start: i64, end: i64, application: u8) -> ActivityInterval {
        ActivityInterval::try_new(
            tracker_run_id(run),
            range(start, end),
            ActivityState::Active,
            ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
                .expect("foreground evidence is valid for the fixture"),
            Some(application_id(application)),
        )
        .expect("active fixture is valid")
    }

    fn state(run: u8, start: i64, end: i64, state: ActivityState) -> ActivityInterval {
        ActivityInterval::try_new(
            tracker_run_id(run),
            range(start, end),
            state,
            ActivityEvidence::try_from_cause(ActivityCause::IdleThreshold)
                .expect("idle evidence is valid for non-powered fixture states"),
            None,
        )
        .expect("non-active fixture is valid")
    }

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("fixture range is positive")
    }

    fn tracker_run_id(value: u8) -> TrackerRunId {
        TrackerRunId::from_bytes([value; 16])
    }

    fn application_id(value: u8) -> ApplicationId {
        ApplicationId::from_bytes([value; 16])
    }

    fn category_id(value: u8) -> CategoryId {
        CategoryId::from_bytes([value; 16])
    }
}
