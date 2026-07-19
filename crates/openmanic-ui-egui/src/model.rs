//! UI-owned state for navigation, presentation states, and correlation-safe updates.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use openmanic_application::{
    AppEvent, ApplicationError, CommandId, DataRevision, EventEnvelope, JobId, JobState,
    MutationOutcome, MutationRejectionReason, ProjectionRequest, ProjectionSlot,
    ProjectionSlotState, SnapshotEnvelope, SnapshotRejection,
};
use openmanic_domain::{ActivityState, ApplicationId, CategoryId, HalfOpenInterval};

use crate::today::TodayAction;

/// A primary destination in the OpenManic shell.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Route {
    /// The daily dashboard and timeline destination.
    Today,
    /// The aggregated time-review destination.
    Overview,
    /// The application-category destination.
    Categories,
    /// The chronological day-review destination.
    Calendar,
    /// The privacy and appearance destination.
    Settings,
}

impl Route {
    /// Returns the stable, ordinary-language navigation label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Overview => "Overview",
            Self::Categories => "Categories",
            Self::Calendar => "Calendar",
            Self::Settings => "Settings",
        }
    }

    /// Returns every primary route in display order.
    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::Today,
            Self::Overview,
            Self::Categories,
            Self::Calendar,
            Self::Settings,
        ]
    }
}

/// Retained UI-only state for a single primary route.
///
/// This state deliberately records navigation and presentation context rather
/// than authoritative activity or persisted settings. Later screen controllers
/// may replace these broad fields with route-specific types without changing
/// the application boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RouteLocalState {
    date_offset_days: i32,
    selected_range: Option<HalfOpenInterval>,
    filter_text: String,
    scroll_anchor: Option<u32>,
}

impl RouteLocalState {
    /// Returns the route's day offset relative to its controller-supplied current day.
    #[must_use]
    pub const fn date_offset_days(&self) -> i32 {
        self.date_offset_days
    }

    /// Returns the retained explicit time selection, if the route has one.
    #[must_use]
    pub const fn selected_range(&self) -> Option<HalfOpenInterval> {
        self.selected_range
    }

    /// Returns the route-local filter draft.
    #[must_use]
    pub fn filter_text(&self) -> &str {
        &self.filter_text
    }

    /// Returns a coarse logical scroll anchor when a controller has recorded one.
    #[must_use]
    pub const fn scroll_anchor(&self) -> Option<u32> {
        self.scroll_anchor
    }
}

/// One category-oriented narrowing criterion for the Today dashboard.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TodayCategoryFilter {
    /// Retains activity associated with this stable category identity.
    Category(CategoryId),
    /// Retains activity whose application has no category assignment.
    Uncategorized,
}

/// Identifies one independently removable Today narrowing criterion.
///
/// A criterion changes only the UI's projection context. It never mutates
/// recorded activity, application assignments, or categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodayNarrowingCriterion {
    /// The shared range selected from the timeline.
    TimelineSelection,
    /// One application identity filter.
    Application(ApplicationId),
    /// One category or uncategorized filter.
    Category(TodayCategoryFilter),
    /// One activity-state filter.
    ActivityState(ActivityState),
}

/// Shared immutable input for every Today dashboard widget in one frame.
///
/// The selected date is represented as an offset from the composition root's
/// current local day. Resolving civil dates and time zones is deliberately an
/// application-boundary concern; the UI never reads a clock or time-zone API.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TodayViewContext {
    selected_day_offset: i32,
    selected_range: Option<HalfOpenInterval>,
    timeline_selection: Option<HalfOpenInterval>,
    application_filter: BTreeSet<ApplicationId>,
    category_filter: BTreeSet<TodayCategoryFilter>,
    activity_state_filter: Vec<ActivityState>,
    revision: u64,
}

impl TodayViewContext {
    /// Returns the selected local-day offset relative to the supplied current day.
    #[must_use]
    pub const fn selected_day_offset(&self) -> i32 {
        self.selected_day_offset
    }

    /// Returns the shared explicit range, when another controller supplied one.
    #[must_use]
    pub const fn selected_range(&self) -> Option<HalfOpenInterval> {
        self.selected_range
    }

    /// Returns the selected timeline range, if any.
    #[must_use]
    pub const fn timeline_selection(&self) -> Option<HalfOpenInterval> {
        self.timeline_selection
    }

    /// Returns the selected application identities in deterministic order.
    #[must_use]
    pub fn application_filter(&self) -> &BTreeSet<ApplicationId> {
        &self.application_filter
    }

    /// Returns the selected categories in deterministic order.
    #[must_use]
    pub fn category_filter(&self) -> &BTreeSet<TodayCategoryFilter> {
        &self.category_filter
    }

    /// Returns the selected activity states in controller insertion order.
    #[must_use]
    pub fn activity_state_filter(&self) -> &[ActivityState] {
        &self.activity_state_filter
    }

    /// Returns the UI-local revision incremented when this projection context changes.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns whether no timeline selection or explicit filters currently narrow widgets.
    #[must_use]
    pub fn has_no_narrowing(&self) -> bool {
        self.timeline_selection.is_none()
            && self.application_filter.is_empty()
            && self.category_filter.is_empty()
            && self.activity_state_filter.is_empty()
    }

    /// Returns every active narrowing criterion in stable display order.
    #[must_use]
    pub fn active_narrowing_criteria(&self) -> Vec<TodayNarrowingCriterion> {
        let mut criteria = Vec::with_capacity(
            usize::from(self.timeline_selection.is_some())
                + self.application_filter.len()
                + self.category_filter.len()
                + self.activity_state_filter.len(),
        );
        if self.timeline_selection.is_some() {
            criteria.push(TodayNarrowingCriterion::TimelineSelection);
        }
        criteria.extend(
            self.application_filter
                .iter()
                .copied()
                .map(TodayNarrowingCriterion::Application),
        );
        criteria.extend(
            self.category_filter
                .iter()
                .copied()
                .map(TodayNarrowingCriterion::Category),
        );
        criteria.extend(
            self.activity_state_filter
                .iter()
                .copied()
                .map(TodayNarrowingCriterion::ActivityState),
        );
        criteria
    }

    pub(crate) fn set_selected_day_offset(&mut self, offset: i32) -> bool {
        let normalized = offset.min(0);
        if self.selected_day_offset == normalized {
            return false;
        }
        self.selected_day_offset = normalized;
        // A selection belongs to the previously selected day/range. Until an
        // application-provided civil-date mapping exists, clearing it is the
        // safe compatibility decision on every day change.
        self.timeline_selection = None;
        self.touch();
        true
    }

    pub(crate) fn set_selected_range(&mut self, range: Option<HalfOpenInterval>) -> bool {
        if self.selected_range == range {
            return false;
        }
        self.selected_range = range;
        self.touch();
        true
    }

    pub(crate) fn set_timeline_selection(&mut self, selection: Option<HalfOpenInterval>) -> bool {
        if self.timeline_selection == selection {
            return false;
        }
        self.timeline_selection = selection;
        self.touch();
        true
    }

    pub(crate) fn add_application_filter(&mut self, application_id: ApplicationId) -> bool {
        if !self.application_filter.insert(application_id) {
            return false;
        }
        self.touch();
        true
    }

    pub(crate) fn remove_application_filter(&mut self, application_id: ApplicationId) -> bool {
        if !self.application_filter.remove(&application_id) {
            return false;
        }
        self.touch();
        true
    }

    pub(crate) fn add_category_filter(&mut self, filter: TodayCategoryFilter) -> bool {
        if !self.category_filter.insert(filter) {
            return false;
        }
        self.touch();
        true
    }

    pub(crate) fn remove_category_filter(&mut self, filter: TodayCategoryFilter) -> bool {
        if !self.category_filter.remove(&filter) {
            return false;
        }
        self.touch();
        true
    }

    pub(crate) fn add_activity_state_filter(&mut self, state: ActivityState) -> bool {
        if self.activity_state_filter.contains(&state) {
            return false;
        }
        self.activity_state_filter.push(state);
        self.touch();
        true
    }

    pub(crate) fn remove_activity_state_filter(&mut self, state: ActivityState) -> bool {
        let Some(index) = self
            .activity_state_filter
            .iter()
            .position(|item| *item == state)
        else {
            return false;
        };
        self.activity_state_filter.remove(index);
        self.touch();
        true
    }

    pub(crate) fn clear_narrowing(&mut self, criterion: TodayNarrowingCriterion) -> bool {
        match criterion {
            TodayNarrowingCriterion::TimelineSelection => self.set_timeline_selection(None),
            TodayNarrowingCriterion::Application(application_id) => {
                self.remove_application_filter(application_id)
            }
            TodayNarrowingCriterion::Category(filter) => self.remove_category_filter(filter),
            TodayNarrowingCriterion::ActivityState(state) => {
                self.remove_activity_state_filter(state)
            }
        }
    }

    pub(crate) fn clear_all_narrowing(&mut self) -> bool {
        if self.has_no_narrowing() {
            return false;
        }
        self.timeline_selection = None;
        self.application_filter.clear();
        self.category_filter.clear();
        self.activity_state_filter.clear();
        self.touch();
        true
    }

    fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

/// Explains why a projection has no values for its requested context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmptyReason {
    /// The requested time range has no recorded values.
    NoRecordedActivity,
    /// Active narrowing criteria exclude all available values.
    NoMatchingResults,
}

impl EmptyReason {
    /// Returns a concise user-facing explanation.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoRecordedActivity => "No activity was recorded for this range.",
            Self::NoMatchingResults => "No results match the current filters.",
        }
    }
}

/// A limitation attached to otherwise usable presentation data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataLimitation {
    /// Tracking was paused for part of the displayed range.
    TrackingPaused,
    /// Tracking was unavailable for part of the displayed range.
    TrackingUnavailable,
    /// Some source data is still being prepared.
    StillLoading,
}

impl DataLimitation {
    /// Returns a concise user-facing explanation.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::TrackingPaused => "Tracking was paused for part of this range.",
            Self::TrackingUnavailable => "Tracking was unavailable for part of this range.",
            Self::StillLoading => "Some values are still loading.",
        }
    }
}

/// A user-safe presentation of an application-boundary failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserFacingError {
    /// A typed application port operation failed.
    Application(ApplicationError),
    /// A mutation was rejected without committing data.
    MutationRejected(MutationRejectionReason),
}

impl UserFacingError {
    /// Returns an ordinary-language recovery message that avoids adapter details.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Application(_) => "The request could not be completed. Try again.".to_owned(),
            Self::MutationRejected(reason) => format!("The change was not saved: {reason}."),
        }
    }
}

/// The explicit presentation state for one immutable data value.
///
/// Refreshing, partial, failed, and recovered variants preserve an immutable
/// prior value whenever one exists. Rendering a state never mutates the value.
#[derive(Clone, Debug)]
pub enum PresentableData<T> {
    /// No snapshot has arrived for the target yet.
    InitialLoading,
    /// A complete immutable value is available.
    Ready(Arc<T>),
    /// A new job is running while a prior immutable value stays visible.
    Refreshing {
        /// The last usable immutable value.
        prior: Arc<T>,
        /// The job producing a replacement value.
        job: JobId,
    },
    /// The request completed successfully but contains no values.
    Empty(EmptyReason),
    /// A usable immutable value carries clear limitations.
    Partial {
        /// The usable immutable value.
        value: Arc<T>,
        /// The conditions limiting interpretation of the value.
        limitations: Vec<DataLimitation>,
    },
    /// The request failed; a recoverable prior value remains available when present.
    Failed {
        /// The last usable immutable value, when one existed before the failure.
        prior: Option<Arc<T>>,
        /// A safe description of the failed operation.
        error: UserFacingError,
    },
    /// A replacement immutable value arrived after a recoverable failure.
    Recovered {
        /// The recovered immutable value.
        value: Arc<T>,
        /// A concise notice that recovery occurred.
        notice: String,
    },
}

impl<T> PresentableData<T> {
    /// Returns the immutable value currently safe to render, if any.
    #[must_use]
    pub fn visible_value(&self) -> Option<&Arc<T>> {
        match self {
            Self::Ready(value)
            | Self::Partial { value, .. }
            | Self::Recovered { value, .. }
            | Self::Refreshing { prior: value, .. } => Some(value),
            Self::Failed { prior, .. } => prior.as_ref(),
            Self::InitialLoading | Self::Empty(_) => None,
        }
    }

    /// Starts a refresh without discarding a usable immutable value.
    #[must_use]
    pub fn refreshing(self, job: JobId) -> Self {
        match self.visible_value() {
            Some(value) => Self::Refreshing {
                prior: Arc::clone(value),
                job,
            },
            None => Self::InitialLoading,
        }
    }

    /// Records a failure without blanking a usable immutable value.
    #[must_use]
    pub fn failed(self, error: UserFacingError) -> Self {
        Self::Failed {
            prior: self.visible_value().map(Arc::clone),
            error,
        }
    }
}

/// The authoritative reconciliation state for one command submitted by this UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationStatus {
    /// The command has been accepted for application handling but is not confirmed.
    Pending,
    /// The command committed at the stated immutable store revision.
    Confirmed {
        /// The revision atomically committed with the mutation.
        data_revision: DataRevision,
    },
    /// The command did not commit and must be reconciled from a future snapshot.
    Rejected {
        /// The typed reason that no authoritative mutation occurred.
        reason: MutationRejectionReason,
    },
}

/// The disposition of an inbound application event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EventReception {
    /// The event updated UI-owned state.
    Applied,
    /// The event sequence was older than an event already observed.
    IgnoredOutOfOrder,
    /// The event did not correlate with a command currently owned by this UI.
    IgnoredUncorrelatedMutation,
    /// The envelope causation metadata disagreed with the mutation payload.
    IgnoredCausationMismatch,
}

/// The result of attempting to install an immutable snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotReception {
    /// The snapshot matched the current target and became its newest value.
    Accepted,
    /// The snapshot was stale or targeted a removed/different slot and was retained nowhere.
    Rejected(SnapshotRejection),
}

#[derive(Debug)]
struct SnapshotTarget<T> {
    state: ProjectionSlotState,
    snapshot: Option<Arc<T>>,
}

/// The UI's complete mutable state, excluding authoritative application data.
///
/// Reducers mutate only UI navigation, presentation, and correlation state.
/// Authoritative facts remain in application-produced immutable snapshots.
#[derive(Debug)]
pub struct UiModel<T> {
    route: Route,
    route_state: RouteStateStore,
    today_view_context: TodayViewContext,
    data: PresentableData<T>,
    mutations: BTreeMap<CommandId, MutationStatus>,
    jobs: BTreeMap<JobId, JobState>,
    projections: BTreeMap<ProjectionSlot, SnapshotTarget<T>>,
    last_event_sequence: Option<u64>,
}

impl<T> Default for UiModel<T> {
    fn default() -> Self {
        Self {
            route: Route::Today,
            route_state: RouteStateStore::default(),
            today_view_context: TodayViewContext::default(),
            data: PresentableData::InitialLoading,
            mutations: BTreeMap::new(),
            jobs: BTreeMap::new(),
            projections: BTreeMap::new(),
            last_event_sequence: None,
        }
    }
}

impl<T> UiModel<T> {
    /// Returns the selected primary route.
    #[must_use]
    pub const fn route(&self) -> Route {
        self.route
    }

    /// Returns retained state for a primary route.
    #[must_use]
    pub fn route_state(&self, route: Route) -> &RouteLocalState {
        self.route_state.get(route)
    }

    /// Returns the shared context consumed by all Today dashboard widgets.
    #[must_use]
    pub const fn today_view_context(&self) -> &TodayViewContext {
        &self.today_view_context
    }

    /// Returns the presentation state for the shell's currently selected data target.
    #[must_use]
    pub const fn data(&self) -> &PresentableData<T> {
        &self.data
    }

    /// Replaces the shell's presentation state with an explicitly derived state.
    pub fn set_data(&mut self, data: PresentableData<T>) {
        self.data = data;
    }

    /// Returns the reconciliation state for a command submitted by this UI.
    #[must_use]
    pub fn mutation_status(&self, command_id: CommandId) -> Option<&MutationStatus> {
        self.mutations.get(&command_id)
    }

    /// Returns the most recently correlated mutation status, if one exists.
    #[must_use]
    pub(crate) fn latest_mutation(&self) -> Option<(CommandId, &MutationStatus)> {
        self.mutations
            .last_key_value()
            .map(|(id, status)| (*id, status))
    }

    /// Returns the latest lifecycle state for a known background job.
    #[must_use]
    pub fn job_state(&self, job_id: JobId) -> Option<&JobState> {
        self.jobs.get(&job_id)
    }

    /// Records that the UI submitted a command and awaits an authoritative outcome.
    pub(crate) fn record_pending_mutation(&mut self, command_id: CommandId) {
        self.mutations.insert(command_id, MutationStatus::Pending);
    }

    /// Begins stale-result tracking for a projection request.
    pub fn begin_projection<K>(&mut self, request: &ProjectionRequest<K>) {
        match self.projections.get_mut(&request.slot()) {
            Some(target) => {
                let _ = target.state.replace_current_request(request);
            }
            None => {
                self.projections.insert(
                    request.slot(),
                    SnapshotTarget {
                        state: ProjectionSlotState::new(request),
                        snapshot: None,
                    },
                );
            }
        }
    }

    /// Applies a snapshot only when its application correlation is still current.
    pub fn accept_snapshot(&mut self, snapshot: &SnapshotEnvelope<T>) -> SnapshotReception {
        let slot = snapshot.slot();
        let Some(target) = self.projections.get_mut(&slot) else {
            return SnapshotReception::Rejected(SnapshotRejection::TargetMissing { slot });
        };
        match target.state.accept_if_current(snapshot) {
            Ok(()) => {
                target.snapshot = Some(snapshot.shared_value());
                SnapshotReception::Accepted
            }
            Err(rejection) => SnapshotReception::Rejected(rejection),
        }
    }

    /// Returns an accepted immutable snapshot for the supplied projection slot.
    #[must_use]
    pub fn snapshot(&self, slot: ProjectionSlot) -> Option<&Arc<T>> {
        self.projections
            .get(&slot)
            .and_then(|target| target.snapshot.as_ref())
    }

    /// Applies one UI-local navigation or presentation action.
    pub fn reduce(&mut self, action: UiAction) {
        match action {
            UiAction::Navigate(route) => self.route = route,
            UiAction::MoveRouteDate { route, days } => {
                if route == Route::Today {
                    let next_offset = self
                        .today_view_context
                        .selected_day_offset()
                        .saturating_add(days);
                    self.set_today_day_offset(next_offset);
                } else {
                    let next_offset = self
                        .route_state(route)
                        .date_offset_days
                        .saturating_add(days);
                    self.route_state_mut(route).date_offset_days = next_offset;
                }
            }
            UiAction::SetRouteRange { route, range } => {
                self.route_state_mut(route).selected_range = range;
                if route == Route::Today {
                    let _ = self.today_view_context.set_selected_range(range);
                }
            }
            UiAction::SetRouteFilter { route, filter } => {
                self.route_state_mut(route).filter_text = filter;
            }
            UiAction::SetRouteScrollAnchor { route, anchor } => {
                self.route_state_mut(route).scroll_anchor = anchor;
            }
            UiAction::Today(action) => self.reduce_today(action),
        }
    }

    pub(crate) fn receive_event(&mut self, event: EventEnvelope<AppEvent>) -> EventReception {
        if self
            .last_event_sequence
            .is_some_and(|last_sequence| event.sequence() <= last_sequence)
        {
            return EventReception::IgnoredOutOfOrder;
        }
        self.last_event_sequence = Some(event.sequence());

        let causation_command_id = event.causation_command_id();
        match event.into_payload() {
            AppEvent::Mutation(outcome)
            | AppEvent::Tracking(openmanic_application::TrackingEvent::Mutation {
                outcome, ..
            }) => self.reconcile_mutation(causation_command_id, outcome),
            AppEvent::Job(job) => {
                self.jobs.insert(job.job_id(), job.state().clone());
                EventReception::Applied
            }
            _ => EventReception::IgnoredUncorrelatedMutation,
        }
    }

    fn reconcile_mutation(
        &mut self,
        causation_command_id: Option<CommandId>,
        outcome: MutationOutcome,
    ) -> EventReception {
        let outcome_command_id = match &outcome {
            MutationOutcome::Confirmed(confirmation) => confirmation.command_id(),
            MutationOutcome::Rejected(rejection) => rejection.command_id(),
        };
        if causation_command_id != Some(outcome_command_id) {
            return EventReception::IgnoredCausationMismatch;
        }
        let Some(status) = self.mutations.get_mut(&outcome_command_id) else {
            return EventReception::IgnoredUncorrelatedMutation;
        };
        *status = match outcome {
            MutationOutcome::Confirmed(confirmation) => MutationStatus::Confirmed {
                data_revision: confirmation.committed_data_revision(),
            },
            MutationOutcome::Rejected(rejection) => MutationStatus::Rejected {
                reason: rejection.reason(),
            },
        };
        EventReception::Applied
    }

    fn route_state_mut(&mut self, route: Route) -> &mut RouteLocalState {
        self.route_state.get_mut(route)
    }

    fn reduce_today(&mut self, action: TodayAction) {
        match action {
            TodayAction::PreviousDay => {
                let previous = self
                    .today_view_context
                    .selected_day_offset()
                    .saturating_sub(1);
                self.set_today_day_offset(previous);
            }
            TodayAction::NextDay => {
                let next = self
                    .today_view_context
                    .selected_day_offset()
                    .saturating_add(1);
                self.set_today_day_offset(next);
            }
            TodayAction::SelectDateOffset { day_offset } => self.set_today_day_offset(day_offset),
            TodayAction::ReturnToToday => self.set_today_day_offset(0),
            TodayAction::SetSharedRange { range } => {
                if self.today_view_context.set_selected_range(range) {
                    self.route_state_mut(Route::Today).selected_range = range;
                }
            }
            TodayAction::SetTimelineSelection { selection } => {
                let _ = self.today_view_context.set_timeline_selection(selection);
            }
            TodayAction::AddApplicationFilter { application_id } => {
                let _ = self
                    .today_view_context
                    .add_application_filter(application_id);
            }
            TodayAction::RemoveApplicationFilter { application_id } => {
                let _ = self
                    .today_view_context
                    .remove_application_filter(application_id);
            }
            TodayAction::AddCategoryFilter { filter } => {
                let _ = self.today_view_context.add_category_filter(filter);
            }
            TodayAction::RemoveCategoryFilter { filter } => {
                let _ = self.today_view_context.remove_category_filter(filter);
            }
            TodayAction::AddActivityStateFilter { state } => {
                let _ = self.today_view_context.add_activity_state_filter(state);
            }
            TodayAction::RemoveActivityStateFilter { state } => {
                let _ = self.today_view_context.remove_activity_state_filter(state);
            }
            TodayAction::ClearNarrowing { criterion } => {
                let _ = self.today_view_context.clear_narrowing(criterion);
            }
            TodayAction::ClearAllNarrowing => {
                let _ = self.today_view_context.clear_all_narrowing();
            }
        }
    }

    fn set_today_day_offset(&mut self, offset: i32) {
        if self.today_view_context.set_selected_day_offset(offset) {
            self.route_state_mut(Route::Today).date_offset_days =
                self.today_view_context.selected_day_offset();
        }
    }
}

#[derive(Debug, Default)]
struct RouteStateStore {
    today: RouteLocalState,
    overview: RouteLocalState,
    categories: RouteLocalState,
    calendar: RouteLocalState,
    settings: RouteLocalState,
}

impl RouteStateStore {
    const fn get(&self, route: Route) -> &RouteLocalState {
        match route {
            Route::Today => &self.today,
            Route::Overview => &self.overview,
            Route::Categories => &self.categories,
            Route::Calendar => &self.calendar,
            Route::Settings => &self.settings,
        }
    }

    fn get_mut(&mut self, route: Route) -> &mut RouteLocalState {
        match route {
            Route::Today => &mut self.today,
            Route::Overview => &mut self.overview,
            Route::Categories => &mut self.categories,
            Route::Calendar => &mut self.calendar,
            Route::Settings => &mut self.settings,
        }
    }
}

/// A UI-local action emitted by shell controls and later screen controllers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiAction {
    /// Select a primary destination without discarding that route's local state.
    Navigate(Route),
    /// Adjust the selected route's controller-defined date by a signed day count.
    MoveRouteDate {
        /// The route whose date navigation is changing.
        route: Route,
        /// The signed number of days to move.
        days: i32,
    },
    /// Replace the selected route's local time range.
    SetRouteRange {
        /// The route whose selection is changing.
        route: Route,
        /// The valid positive interval selected by the user, if any.
        range: Option<HalfOpenInterval>,
    },
    /// Replace the selected route's filter draft.
    SetRouteFilter {
        /// The route whose filter is changing.
        route: Route,
        /// The ordinary-text filter draft.
        filter: String,
    },
    /// Record a coarse scroll anchor suitable for reasonable route restoration.
    SetRouteScrollAnchor {
        /// The route whose scroll context is changing.
        route: Route,
        /// A logical position supplied by the route controller, if known.
        anchor: Option<u32>,
    },
    /// Applies one Today controller action to shared dashboard state.
    Today(TodayAction),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmanic_application::{
        AppEvent, CommandId, DataRevision, EventEnvelope, MutationConfirmation, MutationOutcome,
        MutationRejection, MutationRejectionReason, ProjectionContextKey, ProjectionRequest,
        ProjectionSlot, RequestId, SchemaRevision, SnapshotEnvelope,
    };
    use openmanic_domain::{HalfOpenInterval, UtcMicros};

    use super::{
        EmptyReason, EventReception, MutationStatus, PresentableData, Route, SnapshotReception,
        UiAction, UiModel, UserFacingError,
    };

    #[test]
    fn route_navigation_retains_date_range_filter_and_scroll_state() {
        let mut model = UiModel::<String>::default();
        let range = HalfOpenInterval::try_new(UtcMicros::new(10), UtcMicros::new(20))
            .expect("fixture interval is positive");
        model.reduce(UiAction::MoveRouteDate {
            route: Route::Today,
            days: -2,
        });
        model.reduce(UiAction::SetRouteRange {
            route: Route::Today,
            range: Some(range),
        });
        model.reduce(UiAction::SetRouteFilter {
            route: Route::Today,
            filter: "editor".to_owned(),
        });
        model.reduce(UiAction::SetRouteScrollAnchor {
            route: Route::Today,
            anchor: Some(240),
        });
        model.reduce(UiAction::Navigate(Route::Settings));
        model.reduce(UiAction::Navigate(Route::Today));

        let retained = model.route_state(Route::Today);
        assert_eq!(retained.date_offset_days(), -2);
        assert_eq!(retained.selected_range(), Some(range));
        assert_eq!(retained.filter_text(), "editor");
        assert_eq!(retained.scroll_anchor(), Some(240));
    }

    #[test]
    fn correlated_mutation_outcomes_confirm_and_reject_only_pending_commands() {
        let mut model = UiModel::<String>::default();
        let confirmed = CommandId::new(1);
        let rejected = CommandId::new(2);
        model.record_pending_mutation(confirmed);
        model.record_pending_mutation(rejected);

        assert_eq!(
            model.receive_event(event(
                1,
                confirmed,
                MutationOutcome::Confirmed(MutationConfirmation::new(
                    confirmed,
                    DataRevision::new(4)
                )),
            )),
            EventReception::Applied
        );
        assert_eq!(
            model.mutation_status(confirmed),
            Some(&MutationStatus::Confirmed {
                data_revision: DataRevision::new(4)
            })
        );

        assert_eq!(
            model.receive_event(event(
                2,
                rejected,
                MutationOutcome::Rejected(MutationRejection::new(
                    rejected,
                    MutationRejectionReason::Validation,
                )),
            )),
            EventReception::Applied
        );
        assert_eq!(
            model.mutation_status(rejected),
            Some(&MutationStatus::Rejected {
                reason: MutationRejectionReason::Validation
            })
        );
    }

    #[test]
    fn mismatched_or_stale_events_do_not_replace_command_status() {
        let mut model = UiModel::<String>::default();
        let command_id = CommandId::new(3);
        model.record_pending_mutation(command_id);
        assert_eq!(
            model.receive_event(event(
                5,
                CommandId::new(99),
                MutationOutcome::Confirmed(MutationConfirmation::new(
                    command_id,
                    DataRevision::new(1)
                )),
            )),
            EventReception::IgnoredCausationMismatch
        );
        assert_eq!(
            model.mutation_status(command_id),
            Some(&MutationStatus::Pending)
        );
        assert_eq!(
            model.receive_event(event(
                4,
                command_id,
                MutationOutcome::Confirmed(MutationConfirmation::new(
                    command_id,
                    DataRevision::new(1)
                )),
            )),
            EventReception::IgnoredOutOfOrder
        );
    }

    #[test]
    fn stale_snapshots_do_not_replace_the_newest_immutable_value() {
        let mut model = UiModel::<String>::default();
        let request = request(RequestId::new(10), DataRevision::new(4));
        model.begin_projection(&request);
        assert_eq!(
            model.accept_snapshot(&snapshot(RequestId::new(10), DataRevision::new(5), "new")),
            SnapshotReception::Accepted
        );
        assert_eq!(
            model
                .snapshot(ProjectionSlot::new(7))
                .map(|value| value.as_str()),
            Some("new")
        );
        assert_eq!(
            model.accept_snapshot(&snapshot(RequestId::new(9), DataRevision::new(6), "stale")),
            SnapshotReception::Rejected(
                openmanic_application::SnapshotRejection::RequestNotCurrent {
                    expected: RequestId::new(10),
                    received: RequestId::new(9),
                }
            )
        );
        assert_eq!(
            model
                .snapshot(ProjectionSlot::new(7))
                .map(|value| value.as_str()),
            Some("new")
        );
    }

    #[test]
    fn recoverable_states_keep_the_last_usable_value() {
        let ready = PresentableData::Ready(Arc::new("prior".to_owned()));
        let refreshing = ready.refreshing(openmanic_application::JobId::new(8));
        assert_eq!(
            refreshing.visible_value().map(|value| value.as_str()),
            Some("prior")
        );
        let failed = refreshing.failed(UserFacingError::MutationRejected(
            MutationRejectionReason::ServiceUnavailable,
        ));
        assert_eq!(
            failed.visible_value().map(|value| value.as_str()),
            Some("prior")
        );
        let empty = PresentableData::<String>::Empty(EmptyReason::NoRecordedActivity);
        assert!(empty.visible_value().is_none());
    }

    fn event(
        sequence: u64,
        causation_command_id: CommandId,
        outcome: MutationOutcome,
    ) -> EventEnvelope<AppEvent> {
        EventEnvelope::new(
            SchemaRevision::new(1),
            sequence,
            Some(causation_command_id),
            None,
            UtcMicros::new(0),
            AppEvent::Mutation(outcome),
        )
    }

    fn request(
        request_id: RequestId,
        required_data_revision: DataRevision,
    ) -> ProjectionRequest<()> {
        ProjectionRequest::new(
            request_id,
            ProjectionSlot::new(7),
            ProjectionContextKey::new(9),
            required_data_revision,
            (),
        )
    }

    fn snapshot(
        request_id: RequestId,
        source_data_revision: DataRevision,
        value: &str,
    ) -> SnapshotEnvelope<String> {
        SnapshotEnvelope::new(
            request_id,
            ProjectionSlot::new(7),
            ProjectionContextKey::new(9),
            source_data_revision,
            SchemaRevision::new(1),
            value.to_owned(),
        )
    }
}
