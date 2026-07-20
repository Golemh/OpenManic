//! Cancellable, immutable Overview projection contracts and aggregation.
//!
//! This module owns only the application-layer read model for Overview. Storage
//! supplies a correlated, range-bounded fact slice and the UI consumes the
//! resulting [`SnapshotEnvelope`]. Saved views and rendering deliberately live
//! in later Phase 4 tasks.

use core::fmt;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use jiff::civil::Date;
use openmanic_domain::{ApplicationId, CategoryId, HalfOpenInterval};

use crate::{
    CancellationToken, DataRevision, ProjectionContextKey, ProjectionRequest, ProjectionSlotState,
    SchemaRevision, SnapshotEnvelope, SnapshotRejection,
};

/// Schema revision for immutable Overview snapshots.
pub const OVERVIEW_SNAPSHOT_SCHEMA_REVISION: SchemaRevision = SchemaRevision::new(1);

/// A local-calendar Overview range with its already-resolved UTC boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OverviewRange {
    /// One local day.
    Day {
        /// The local civil date selected by the user.
        date: Date,
        /// The exact UTC boundaries resolved for that local date.
        utc_range: HalfOpenInterval,
    },
    /// One local week, identified by its first local date.
    Week {
        /// The first local civil date in the selected week.
        first_date: Date,
        /// The exact UTC boundaries resolved for that local week.
        utc_range: HalfOpenInterval,
    },
    /// One local month.
    Month {
        /// A local civil date within the selected month.
        date: Date,
        /// The exact UTC boundaries resolved for that local month.
        utc_range: HalfOpenInterval,
    },
    /// One local year.
    Year {
        /// A local civil date within the selected year.
        date: Date,
        /// The exact UTC boundaries resolved for that local year.
        utc_range: HalfOpenInterval,
    },
    /// An inclusive fixed local-date range.
    Custom {
        /// The first inclusive local civil date.
        first_date: Date,
        /// The last inclusive local civil date.
        last_date: Date,
        /// The exact UTC boundaries resolved for the local-date range.
        utc_range: HalfOpenInterval,
    },
}

impl OverviewRange {
    /// Returns the exact resolved UTC half-open range used for aggregation.
    #[must_use]
    pub const fn utc_range(self) -> HalfOpenInterval {
        match self {
            Self::Day { utc_range, .. }
            | Self::Week { utc_range, .. }
            | Self::Month { utc_range, .. }
            | Self::Year { utc_range, .. }
            | Self::Custom { utc_range, .. } => utc_range,
        }
    }
}

/// A shared selected range from a compatible Overview or Timeline surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SharedOverviewSelection(HalfOpenInterval);

impl SharedOverviewSelection {
    /// Creates a selection from a positive half-open interval.
    #[must_use]
    pub const fn new(range: HalfOpenInterval) -> Self {
        Self(range)
    }

    /// Returns the selected interval.
    #[must_use]
    pub const fn range(self) -> HalfOpenInterval {
        self.0
    }
}

/// Stable, duplicate-free application and category narrowing criteria.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct OverviewFilters {
    application_ids: BTreeSet<ApplicationId>,
    category_ids: BTreeSet<CategoryId>,
}

impl OverviewFilters {
    /// Normalizes unordered application/category input into deterministic filter sets.
    #[must_use]
    pub fn new(
        application_ids: impl IntoIterator<Item = ApplicationId>,
        category_ids: impl IntoIterator<Item = CategoryId>,
    ) -> Self {
        Self {
            application_ids: application_ids.into_iter().collect(),
            category_ids: category_ids.into_iter().collect(),
        }
    }

    /// Returns normalized application filters.
    #[must_use]
    pub const fn application_ids(&self) -> &BTreeSet<ApplicationId> {
        &self.application_ids
    }

    /// Returns normalized category filters.
    #[must_use]
    pub const fn category_ids(&self) -> &BTreeSet<CategoryId> {
        &self.category_ids
    }

    fn matches(&self, activity: OverviewSourceActivity) -> bool {
        (self.application_ids.is_empty()
            || activity
                .application_id()
                .is_some_and(|id| self.application_ids.contains(&id)))
            && (self.category_ids.is_empty()
                || activity
                    .category_id()
                    .is_some_and(|id| self.category_ids.contains(&id)))
    }
}

/// The allocation dimension selected for the Overview.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OverviewGrouping {
    /// Aggregate activity by application identity.
    Application,
    /// Aggregate activity by category identity.
    Category,
}

/// Fully normalized, immutable request payload shared by all compatible Overview widgets.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OverviewContext {
    context_key: ProjectionContextKey,
    range: OverviewRange,
    selection: Option<SharedOverviewSelection>,
    filters: OverviewFilters,
    grouping: OverviewGrouping,
}

impl OverviewContext {
    /// Creates a context, clipping a compatible selection to the active range.
    #[must_use]
    pub fn new(
        context_key: ProjectionContextKey,
        range: OverviewRange,
        selection: Option<SharedOverviewSelection>,
        filters: OverviewFilters,
        grouping: OverviewGrouping,
    ) -> Self {
        Self {
            context_key,
            range,
            selection: selection.and_then(|value| {
                intersect(range.utc_range(), value.range()).map(SharedOverviewSelection::new)
            }),
            filters,
            grouping,
        }
    }

    /// Returns the normalized correlation key.
    #[must_use]
    pub const fn context_key(&self) -> ProjectionContextKey {
        self.context_key
    }
    /// Returns the active local-calendar range.
    #[must_use]
    pub const fn range(&self) -> OverviewRange {
        self.range
    }
    /// Returns the compatible clipped shared selection.
    #[must_use]
    pub const fn selection(&self) -> Option<SharedOverviewSelection> {
        self.selection
    }
    /// Returns stable explicit filters.
    #[must_use]
    pub const fn filters(&self) -> &OverviewFilters {
        &self.filters
    }
    /// Returns the chosen allocation dimension.
    #[must_use]
    pub const fn grouping(&self) -> OverviewGrouping {
        self.grouping
    }
    /// Returns the exact aggregation range after shared-selection intersection.
    #[must_use]
    pub const fn effective_range(&self) -> HalfOpenInterval {
        match self.selection {
            Some(value) => value.range(),
            None => self.range.utc_range(),
        }
    }
}

/// Cache identity which cannot be reused after a committed data revision changes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OverviewCacheKey {
    context: OverviewContext,
    data_revision: DataRevision,
}

impl OverviewCacheKey {
    /// Creates a revision-correlated cache key.
    #[must_use]
    pub const fn new(context: OverviewContext, data_revision: DataRevision) -> Self {
        Self {
            context,
            data_revision,
        }
    }
}

/// One storage-selected, correlated fact for Overview aggregation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverviewSourceActivity {
    interval: HalfOpenInterval,
    application_id: Option<ApplicationId>,
    category_id: Option<CategoryId>,
}

impl OverviewSourceActivity {
    /// Creates a source activity fact.
    #[must_use]
    pub const fn new(
        interval: HalfOpenInterval,
        application_id: Option<ApplicationId>,
        category_id: Option<CategoryId>,
    ) -> Self {
        Self {
            interval,
            application_id,
            category_id,
        }
    }
    /// Returns the canonical activity range.
    #[must_use]
    pub const fn interval(self) -> HalfOpenInterval {
        self.interval
    }
    /// Returns its application identity when known.
    #[must_use]
    pub const fn application_id(self) -> Option<ApplicationId> {
        self.application_id
    }
    /// Returns its category identity when known.
    #[must_use]
    pub const fn category_id(self) -> Option<CategoryId> {
        self.category_id
    }
}

/// Borrowed source facts from one atomic data revision.
#[derive(Clone, Copy, Debug)]
pub struct OverviewProjectionSource<'a> {
    data_revision: DataRevision,
    activities: &'a [OverviewSourceActivity],
}

impl<'a> OverviewProjectionSource<'a> {
    /// Creates a correlated source view.
    #[must_use]
    pub const fn new(
        data_revision: DataRevision,
        activities: &'a [OverviewSourceActivity],
    ) -> Self {
        Self {
            data_revision,
            activities,
        }
    }
}

/// Stable aggregate bucket identity, including unassigned data.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OverviewAllocationIdentity {
    /// A known application allocation.
    Application(ApplicationId),
    /// Activity without an associated application.
    UnassignedApplication,
    /// A known category allocation.
    Category(CategoryId),
    /// Activity without an associated category.
    Uncategorized,
}

/// One immutable allocation value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverviewAllocation {
    identity: OverviewAllocationIdentity,
    duration_us: u64,
    percentage_basis_points: u16,
}

impl OverviewAllocation {
    /// Returns the allocation bucket.
    #[must_use]
    pub const fn identity(self) -> OverviewAllocationIdentity {
        self.identity
    }
    /// Returns the exact accumulated duration.
    #[must_use]
    pub const fn duration_us(self) -> u64 {
        self.duration_us
    }
    /// Returns the deterministic truncated proportion out of 10,000.
    #[must_use]
    pub const fn percentage_basis_points(self) -> u16 {
        self.percentage_basis_points
    }
}

/// Presentation-ready immutable Overview result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewSnapshot {
    context: OverviewContext,
    source_data_revision: DataRevision,
    total_duration_us: u64,
    allocations: Arc<[OverviewAllocation]>,
}

impl OverviewSnapshot {
    /// Returns the normalized context.
    #[must_use]
    pub const fn context(&self) -> &OverviewContext {
        &self.context
    }
    /// Returns the correlated source revision.
    #[must_use]
    pub const fn source_data_revision(&self) -> DataRevision {
        self.source_data_revision
    }
    /// Returns the exact filtered total used for percentage calculation.
    #[must_use]
    pub const fn total_duration_us(&self) -> u64 {
        self.total_duration_us
    }
    /// Returns allocations in deterministic descending-duration order.
    #[must_use]
    pub fn allocations(&self) -> &[OverviewAllocation] {
        &self.allocations
    }
}

/// Cooperative projection result; cancellation never produces a publishable snapshot.
#[derive(Clone, Debug)]
pub enum OverviewProjectionResult {
    /// A fully prepared, publishable immutable snapshot.
    Completed(SnapshotEnvelope<OverviewSnapshot>),
    /// A superseded request that deliberately has no snapshot to publish.
    Cancelled,
}

/// Pure immutable Overview aggregation.
pub struct OverviewProjector;

impl OverviewProjector {
    /// Projects a request using cancellation checkpoints between source facts.
    ///
    /// # Errors
    ///
    /// Returns an error when context keys differ or durations overflow.
    pub fn project(
        request: &ProjectionRequest<OverviewContext>,
        source: OverviewProjectionSource<'_>,
        cancellation: &CancellationToken,
    ) -> Result<OverviewProjectionResult, OverviewProjectionError> {
        if request.context_key() != request.payload().context_key() {
            return Err(OverviewProjectionError::ContextKeyMismatch {
                request: request.context_key(),
                context: request.payload().context_key(),
            });
        }
        let context = request.payload();
        let mut buckets = BTreeMap::new();
        for activity in source.activities {
            if cancellation.is_cancelled() {
                return Ok(OverviewProjectionResult::Cancelled);
            }
            if !context.filters.matches(*activity) {
                continue;
            }
            let Some(clipped) = intersect(activity.interval(), context.effective_range()) else {
                continue;
            };
            let total = buckets
                .entry(bucket(context.grouping(), *activity))
                .or_insert(0_u64);
            *total = total
                .checked_add(clipped.duration_us())
                .ok_or(OverviewProjectionError::DurationOverflow)?;
        }
        if cancellation.is_cancelled() {
            return Ok(OverviewProjectionResult::Cancelled);
        }
        let total_duration_us = buckets.values().try_fold(0_u64, |total, value| {
            total
                .checked_add(*value)
                .ok_or(OverviewProjectionError::DurationOverflow)
        })?;
        let mut allocations: Vec<_> = buckets
            .into_iter()
            .map(|(identity, duration_us)| OverviewAllocation {
                identity,
                duration_us,
                percentage_basis_points: percentage(duration_us, total_duration_us),
            })
            .collect();
        allocations.sort_unstable_by(|left, right| {
            right
                .duration_us
                .cmp(&left.duration_us)
                .then_with(|| left.identity.cmp(&right.identity))
        });
        Ok(OverviewProjectionResult::Completed(SnapshotEnvelope::new(
            request.request_id(),
            request.slot(),
            request.context_key(),
            source.data_revision,
            OVERVIEW_SNAPSHOT_SCHEMA_REVISION,
            OverviewSnapshot {
                context: context.clone(),
                source_data_revision: source.data_revision,
                total_duration_us,
                allocations: Arc::from(allocations),
            },
        )))
    }
}

/// Explicit delivery status retaining immutable prior data during progress or cancellation.
#[derive(Clone, Debug)]
pub enum OverviewProjectionStatus {
    /// No snapshot has been accepted for this slot.
    Initial,
    /// A new request is pending while an older compatible snapshot remains visible.
    Progressive {
        /// The retained immutable prior snapshot.
        prior: Arc<OverviewSnapshot>,
    },
    /// The current request has an accepted immutable snapshot.
    Ready {
        /// The current accepted immutable snapshot.
        snapshot: Arc<OverviewSnapshot>,
    },
    /// The current request was cancelled without publishing data.
    Cancelled {
        /// An optionally retained immutable prior snapshot.
        prior: Option<Arc<OverviewSnapshot>>,
    },
}

/// Overview-specific delivery state built on generic stale-result protection.
#[derive(Clone, Debug)]
pub struct OverviewProjectionSlotState {
    slot: ProjectionSlotState,
    status: OverviewProjectionStatus,
}

impl OverviewProjectionSlotState {
    /// Starts with an initial loading state.
    #[must_use]
    pub fn new(request: &ProjectionRequest<OverviewContext>) -> Self {
        Self {
            slot: ProjectionSlotState::new(request),
            status: OverviewProjectionStatus::Initial,
        }
    }
    /// Begins a current request while preserving any accepted snapshot as progressive data.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRejection`] when the request targets a different projection slot.
    pub fn replace_current_request(
        &mut self,
        request: &ProjectionRequest<OverviewContext>,
    ) -> Result<(), SnapshotRejection> {
        self.slot.replace_current_request(request)?;
        self.status = match &self.status {
            OverviewProjectionStatus::Ready { snapshot }
            | OverviewProjectionStatus::Progressive { prior: snapshot }
            | OverviewProjectionStatus::Cancelled {
                prior: Some(snapshot),
            } => OverviewProjectionStatus::Progressive {
                prior: Arc::clone(snapshot),
            },
            OverviewProjectionStatus::Initial
            | OverviewProjectionStatus::Cancelled { prior: None } => {
                OverviewProjectionStatus::Initial
            }
        };
        Ok(())
    }
    /// Accepts only generic-current, context-matching, non-stale completed snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRejection`] when the snapshot is stale, mismatched, or targets a
    /// missing slot.
    pub fn accept_completed(
        &mut self,
        snapshot: &SnapshotEnvelope<OverviewSnapshot>,
    ) -> Result<(), SnapshotRejection> {
        self.slot.accept_if_current(snapshot)?;
        self.status = OverviewProjectionStatus::Ready {
            snapshot: snapshot.shared_value(),
        };
        Ok(())
    }
    /// Records cancellation without accepting or replacing immutable prior data.
    pub fn record_cancelled(&mut self) {
        let prior = match &self.status {
            OverviewProjectionStatus::Ready { snapshot }
            | OverviewProjectionStatus::Progressive { prior: snapshot }
            | OverviewProjectionStatus::Cancelled {
                prior: Some(snapshot),
            } => Some(Arc::clone(snapshot)),
            OverviewProjectionStatus::Initial
            | OverviewProjectionStatus::Cancelled { prior: None } => None,
        };
        self.status = OverviewProjectionStatus::Cancelled { prior };
    }
    /// Returns the explicit current status.
    #[must_use]
    pub const fn status(&self) -> &OverviewProjectionStatus {
        &self.status
    }
}

/// Projection failure which cannot be represented as a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverviewProjectionError {
    /// The request and its normalized payload name different projection contexts.
    ContextKeyMismatch {
        /// The correlation key carried by the request envelope.
        request: ProjectionContextKey,
        /// The correlation key embedded in the Overview context.
        context: ProjectionContextKey,
    },
    /// Summing clipped activity durations exceeded the supported total.
    DurationOverflow,
}

impl fmt::Display for OverviewProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContextKeyMismatch { request, context } => write!(
                formatter,
                "overview context key {} does not match request key {}",
                context.get(),
                request.get()
            ),
            Self::DurationOverflow => formatter.write_str("overview duration total overflowed"),
        }
    }
}
impl std::error::Error for OverviewProjectionError {}

fn bucket(
    grouping: OverviewGrouping,
    activity: OverviewSourceActivity,
) -> OverviewAllocationIdentity {
    match grouping {
        OverviewGrouping::Application => activity.application_id().map_or(
            OverviewAllocationIdentity::UnassignedApplication,
            OverviewAllocationIdentity::Application,
        ),
        OverviewGrouping::Category => activity.category_id().map_or(
            OverviewAllocationIdentity::Uncategorized,
            OverviewAllocationIdentity::Category,
        ),
    }
}
fn intersect(left: HalfOpenInterval, right: HalfOpenInterval) -> Option<HalfOpenInterval> {
    HalfOpenInterval::try_new(left.start().max(right.start()), left.end().min(right.end())).ok()
}
fn percentage(duration: u64, total: u64) -> u16 {
    if total == 0 {
        0
    } else {
        ((u128::from(duration) * 10_000) / u128::from(total))
            .try_into()
            .unwrap_or(u16::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CancellationRequest, CancellationSource, ProjectionSlot, RequestId};
    use openmanic_domain::UtcMicros;

    #[test]
    fn ranges_filters_and_cache_keys_are_normalized() {
        let utc_range = interval(10, 40);
        assert_eq!(
            OverviewRange::Day {
                date: date(),
                utc_range
            }
            .utc_range(),
            utc_range
        );
        assert_eq!(
            OverviewRange::Week {
                first_date: date(),
                utc_range
            }
            .utc_range(),
            utc_range
        );
        assert_eq!(
            OverviewRange::Month {
                date: date(),
                utc_range
            }
            .utc_range(),
            utc_range
        );
        assert_eq!(
            OverviewRange::Year {
                date: date(),
                utc_range
            }
            .utc_range(),
            utc_range
        );
        assert_eq!(
            OverviewRange::Custom {
                first_date: date(),
                last_date: date(),
                utc_range
            }
            .utc_range(),
            utc_range
        );
        let context = OverviewContext::new(
            ProjectionContextKey::new(1),
            range(),
            Some(SharedOverviewSelection::new(interval(10, 20))),
            OverviewFilters::new(
                [application(2), application(1), application(1)],
                [category(3), category(3)],
            ),
            OverviewGrouping::Application,
        );
        assert_eq!(context.effective_range(), interval(10, 20));
        assert_eq!(context.filters().application_ids().len(), 2);
        assert_ne!(
            OverviewCacheKey::new(context.clone(), DataRevision::new(4)),
            OverviewCacheKey::new(context, DataRevision::new(5))
        );
    }

    #[test]
    fn aggregation_is_deterministic_and_cancellation_preserves_prior_snapshot() {
        let context = context(OverviewGrouping::Application);
        let activities = [
            activity(0, 20, Some(2), Some(8)),
            activity(20, 40, Some(1), Some(8)),
            activity(40, 50, None, None),
        ];
        let first = request(1, context.clone(), 3);
        let (_source, token) = CancellationSource::new();
        let result = OverviewProjector::project(
            &first,
            OverviewProjectionSource::new(DataRevision::new(3), &activities),
            &token,
        )
        .expect("finite fixture projects");
        assert!(matches!(result, OverviewProjectionResult::Completed(_)));
        let OverviewProjectionResult::Completed(snapshot) = result else {
            return;
        };
        assert_eq!(snapshot.value().total_duration_us(), 40);
        assert_eq!(
            snapshot.value().allocations()[0].identity(),
            OverviewAllocationIdentity::Application(application(1))
        );
        assert_eq!(
            snapshot.value().allocations()[0].percentage_basis_points(),
            5_000
        );
        let mut state = OverviewProjectionSlotState::new(&first);
        state
            .accept_completed(&snapshot)
            .expect("first snapshot is current");
        let current = request(2, context, 4);
        state
            .replace_current_request(&current)
            .expect("same slot is current");
        assert!(matches!(
            state.status(),
            OverviewProjectionStatus::Progressive { .. }
        ));
        assert!(matches!(
            state.accept_completed(&snapshot),
            Err(SnapshotRejection::RequestNotCurrent { .. })
        ));
        let (source, cancelled) = CancellationSource::new();
        assert_eq!(source.cancel(), CancellationRequest::Requested);
        assert!(matches!(
            OverviewProjector::project(
                &current,
                OverviewProjectionSource::new(DataRevision::new(4), &activities),
                &cancelled
            )
            .expect("cancellation is ordinary"),
            OverviewProjectionResult::Cancelled
        ));
        state.record_cancelled();
        assert!(matches!(
            state.status(),
            OverviewProjectionStatus::Cancelled { prior: Some(_) }
        ));
    }

    fn context(grouping: OverviewGrouping) -> OverviewContext {
        OverviewContext::new(
            ProjectionContextKey::new(9),
            range(),
            None,
            OverviewFilters::default(),
            grouping,
        )
    }
    fn request(
        id: u64,
        context: OverviewContext,
        revision: u64,
    ) -> ProjectionRequest<OverviewContext> {
        ProjectionRequest::new(
            RequestId::new(id),
            ProjectionSlot::new(7),
            context.context_key(),
            DataRevision::new(revision),
            context,
        )
    }
    fn range() -> OverviewRange {
        OverviewRange::Day {
            date: date(),
            utc_range: interval(0, 40),
        }
    }
    fn date() -> Date {
        Date::new(2026, 7, 20).expect("valid fixture date")
    }
    fn interval(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("positive fixture range")
    }
    fn activity(
        start: i64,
        end: i64,
        application_id: Option<u8>,
        category_id: Option<u8>,
    ) -> OverviewSourceActivity {
        OverviewSourceActivity::new(
            interval(start, end),
            application_id.map(application),
            category_id.map(category),
        )
    }
    fn application(value: u8) -> ApplicationId {
        ApplicationId::from_bytes([value; 16])
    }
    fn category(value: u8) -> CategoryId {
        CategoryId::from_bytes([value; 16])
    }
}
