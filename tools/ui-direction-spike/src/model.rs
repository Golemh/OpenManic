//! Deterministic local data and reducer state for the UI direction spike.

use fixture_generator::{
    Scenario,
    scenarios::{OverlayKind, ScheduleMarker},
};
use std::collections::BTreeSet;

const TIMELINE_DURATION: f32 = 120.0;
const FIXTURE_SEED: u64 = 2_026_050;

/// A primary destination available in the direction review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Route {
    /// The daily dashboard, including the central Timeline flow.
    Today,
    /// Aggregated time-review controls.
    Overview,
    /// Application category assignment controls.
    Categories,
    /// A day-oriented chronological review.
    Calendar,
    /// Privacy and appearance controls.
    Settings,
}

impl Route {
    /// Returns the route label used in navigation.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Overview => "Overview",
            Self::Categories => "Categories",
            Self::Calendar => "Calendar",
            Self::Settings => "Settings",
        }
    }

    /// Returns each primary route in display order.
    #[must_use]
    pub(crate) const fn all() -> [Self; 5] {
        [
            Self::Today,
            Self::Overview,
            Self::Categories,
            Self::Calendar,
            Self::Settings,
        ]
    }
}

/// A deterministic presentational state selectable during review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MockDataState {
    /// No result has arrived yet.
    InitialLoading,
    /// A complete snapshot is shown.
    Ready,
    /// A valid prior snapshot remains visible while refresh occurs.
    Refreshing,
    /// The requested context has no records.
    Empty,
    /// Valid values carry a clear limitation.
    Partial,
    /// The request failed but can be retried.
    Failed,
    /// A recovered value carries a visible notice.
    Recovered,
}

impl MockDataState {
    /// Returns the short state label.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::InitialLoading => "Loading",
            Self::Ready => "Ready",
            Self::Refreshing => "Refreshing",
            Self::Empty => "Empty",
            Self::Partial => "Partial",
            Self::Failed => "Error",
            Self::Recovered => "Recovered",
        }
    }

    /// Returns every review state in a stable order.
    #[must_use]
    pub(crate) const fn all() -> [Self; 7] {
        [
            Self::InitialLoading,
            Self::Ready,
            Self::Refreshing,
            Self::Empty,
            Self::Partial,
            Self::Failed,
            Self::Recovered,
        ]
    }
}

/// A replaceable visual representation for the distribution widget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DistributionPresentation {
    /// A labeled proportional stacked bar. This is the provisional recommendation.
    StackedBar,
    /// A compact ring with a conventional text legend.
    Ring,
}

impl DistributionPresentation {
    /// Returns the user-facing presentation name.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::StackedBar => "Stacked bar",
            Self::Ring => "Ring",
        }
    }
}

/// A fixed logical-width review viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewWidth {
    /// The MVP's compact desktop width.
    Compact,
    /// The MVP's standard desktop width.
    Standard,
    /// The MVP's wide desktop width.
    Wide,
}

impl PreviewWidth {
    /// Returns the represented logical pixel width.
    #[must_use]
    pub(crate) const fn logical_pixels(self) -> u16 {
        match self {
            Self::Compact => 720,
            Self::Standard => 1_024,
            Self::Wide => 1_440,
        }
    }

    /// Returns every supported review width.
    #[must_use]
    pub(crate) const fn all() -> [Self; 3] {
        [Self::Compact, Self::Standard, Self::Wide]
    }
}

/// A broad overview range used by the local saved-view flow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OverviewRange {
    /// One selected day.
    Day,
    /// One calendar week.
    Week,
    /// One calendar month.
    Month,
    /// One calendar year.
    Year,
    /// A local explicit range.
    Custom,
}

impl OverviewRange {
    /// Returns the range label.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Year => "Year",
            Self::Custom => "Custom",
        }
    }

    /// Returns all range variants.
    #[must_use]
    pub(crate) const fn all() -> [Self; 5] {
        [Self::Day, Self::Week, Self::Month, Self::Year, Self::Custom]
    }
}

/// A local three-band identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BandKind {
    /// Category allocation.
    Category,
    /// Explicit tracking state.
    ActivityState,
    /// Foreground application identity.
    Application,
}

impl BandKind {
    /// Returns the visible band label.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Category => "Category",
            Self::ActivityState => "Tracking state",
            Self::Application => "Application",
        }
    }
}

/// An exact interval within the review-only timeline coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TimeRange {
    /// Inclusive start in mock seconds from the timeline origin.
    pub start: f32,
    /// Exclusive end in mock seconds from the timeline origin.
    pub end: f32,
}

impl TimeRange {
    /// Creates an ordered range, accepting either drag direction.
    #[must_use]
    pub(crate) fn ordered(left: f32, right: f32) -> Self {
        Self {
            start: left.min(right),
            end: left.max(right),
        }
    }

    /// Returns whether the range contains meaningful width.
    #[must_use]
    pub(crate) fn is_meaningful(self) -> bool {
        self.end - self.start >= 1.0
    }
}

/// A display-ready timeline segment derived from a deterministic fixture.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MockSegment {
    /// Stable fixture segment identity.
    pub id: u64,
    /// Presentation band that owns the segment.
    pub band: BandKind,
    /// Short user-facing identity.
    pub label: String,
    /// Inclusive mock offset in seconds.
    pub start: f32,
    /// Exclusive mock offset in seconds.
    pub end: f32,
    /// Whether the segment is painted as explicit but intentionally unfilled.
    pub unfilled: bool,
}

/// A timeline overlay represented separately from recorded activity.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MockOverlay {
    /// Stable local overlay identity.
    pub id: u64,
    /// Brief user-facing label.
    pub label: String,
    /// Interval placement in mock seconds.
    pub range: TimeRange,
    /// Whether this is a personal schedule bracket rather than a focus overlay.
    pub schedule: bool,
}

/// Data used by one display band.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MockBand {
    /// Band identity.
    pub kind: BandKind,
    /// Independently segmented values.
    pub segments: Vec<MockSegment>,
}

/// The synthetic timeline data used by the spike.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MockTimeline {
    /// Independent category boundaries.
    pub category: MockBand,
    /// Independent tracking-state boundaries.
    pub state: MockBand,
    /// Independent application boundaries.
    pub application: MockBand,
    /// Schedule and focus overlays.
    pub overlays: Vec<MockOverlay>,
}

/// A review-only application row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MockApplication {
    /// User-facing name.
    pub name: String,
    /// Deterministic duration text.
    pub duration: String,
    /// Included-time percentage.
    pub percent: u8,
    /// Current category assignment.
    pub category: String,
}

/// Immutable mock data collected from OM-030 fixture scenarios.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MockSnapshot {
    /// Three independently segmented bands and their overlays.
    pub timeline: MockTimeline,
    /// Application list and usage-widget rows.
    pub applications: Vec<MockApplication>,
    /// Stable category names available to local assignment controls.
    pub categories: Vec<String>,
    /// A review-only source statement.
    pub source_note: String,
    /// Original dense-fixture count, retained as a review disclosure.
    pub dense_interval_count: usize,
}

impl MockSnapshot {
    /// Builds deterministic review data from the already accepted fixture-generator APIs.
    #[must_use]
    pub(crate) fn deterministic() -> Self {
        let bands_fixture = Scenario::ThreeSegmentedBands.generate(FIXTURE_SEED);
        let normal_fixture = Scenario::NormalWorkday.generate(FIXTURE_SEED);
        let schedule_fixture = Scenario::ScheduleDstOvernight.generate(FIXTURE_SEED);
        let overlay_fixture = Scenario::SimultaneousOverlays.generate(FIXTURE_SEED);
        let dense_fixture = Scenario::Dense10000IntervalRange.generate(FIXTURE_SEED);
        let origin = bands_fixture
            .category_band
            .first()
            .map_or(0_i64, |segment| segment.start.get());

        let category = MockBand {
            kind: BandKind::Category,
            segments: bands_fixture
                .category_band
                .iter()
                .map(|segment| {
                    mock_segment(
                        segment.id,
                        BandKind::Category,
                        &segment.label,
                        segment.start.get(),
                        segment.end.get(),
                        origin,
                    )
                })
                .collect(),
        };
        let state = MockBand {
            kind: BandKind::ActivityState,
            segments: bands_fixture
                .state_band
                .iter()
                .enumerate()
                .map(|(index, segment)| {
                    let label = if index + 1 == bands_fixture.state_band.len() {
                        "Powered off"
                    } else {
                        segment.label.as_str()
                    };
                    mock_segment(
                        segment.id,
                        BandKind::ActivityState,
                        label,
                        segment.start.get(),
                        segment.end.get(),
                        origin,
                    )
                })
                .collect(),
        };
        let application = MockBand {
            kind: BandKind::Application,
            segments: bands_fixture
                .application_band
                .iter()
                .map(|segment| {
                    mock_segment(
                        segment.id,
                        BandKind::Application,
                        &segment.label,
                        segment.start.get(),
                        segment.end.get(),
                        origin,
                    )
                })
                .collect(),
        };
        let schedules =
            schedule_fixture
                .schedules
                .iter()
                .take(3)
                .enumerate()
                .map(|(index, item)| MockOverlay {
                    id: item.id,
                    label: schedule_label(&item.marker).to_owned(),
                    range: TimeRange {
                        start: 12.0 + index as f32 * 28.0,
                        end: 32.0 + index as f32 * 28.0,
                    },
                    schedule: true,
                });
        let focus = overlay_fixture
            .overlays
            .iter()
            .find(|item| matches!(item.kind, OverlayKind::Focus));
        let focus_overlay = focus.map(|item| MockOverlay {
            id: item.id.saturating_add(100),
            label: "Focus session".to_owned(),
            range: TimeRange {
                start: 76.0,
                end: 100.0,
            },
            schedule: false,
        });
        let applications = normal_fixture
            .activity_intervals
            .iter()
            .take(4)
            .enumerate()
            .map(|(index, interval)| MockApplication {
                name: format!("Application {}", index + 1),
                duration: format!("{} min", 48_u8.saturating_sub(index as u8 * 9)),
                percent: 38_u8.saturating_sub(index as u8 * 7),
                category: interval.category_id.clone(),
            })
            .collect();

        Self {
            timeline: MockTimeline {
                category,
                state,
                application,
                overlays: schedules.chain(focus_overlay).collect(),
            },
            applications,
            categories: vec![
                "Productive".to_owned(),
                "Communication".to_owned(),
                "Entertainment".to_owned(),
                "Uncategorized".to_owned(),
            ],
            source_note:
                "Derived locally from OM-030 deterministic fixture scenarios; not production data."
                    .to_owned(),
            dense_interval_count: dense_fixture.metadata.raw_interval_count,
        }
    }
}

fn mock_segment(
    id: u64,
    band: BandKind,
    label: &str,
    start: i64,
    end: i64,
    origin: i64,
) -> MockSegment {
    MockSegment {
        id,
        band,
        label: label.to_owned(),
        start: ((start - origin) as f32 / 1_000_000.0).clamp(0.0, TIMELINE_DURATION),
        end: ((end - origin) as f32 / 1_000_000.0).clamp(0.0, TIMELINE_DURATION),
        unfilled: label == "Powered off",
    }
}

fn schedule_label(marker: &ScheduleMarker) -> &'static str {
    match marker {
        ScheduleMarker::Adjacent => "Planned focus",
        ScheduleMarker::Overnight => "Overnight personal time",
        ScheduleMarker::DstBoundary => "Schedule boundary",
        ScheduleMarker::RecurrenceException => "Schedule exception",
    }
}

/// A transient segment identity used by hover and persistent details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MockSegmentRef {
    /// Band identity.
    pub band: BandKind,
    /// Stable segment identifier.
    pub id: u64,
    /// User-facing value label.
    pub label: String,
}

/// A persistent local selection.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MockSelection {
    /// One selected band value.
    Segment(MockSegmentRef),
    /// A selected time range.
    Range(TimeRange),
}

/// Transient timeline controller state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimelineState {
    /// Visible-range start in mock seconds.
    pub view_start: f32,
    /// Visible duration in mock seconds.
    pub view_span: f32,
    /// The one current hover target.
    pub hover: Option<MockSegmentRef>,
    /// Persistent selected value or range.
    pub selection: Option<MockSelection>,
    /// Drag start, while a single interaction response is captured.
    pub drag_origin: Option<f32>,
    /// Explicit mode that changes drag from range-select to schedule draft.
    pub create_schedule_mode: bool,
    /// A local unsaved bracket preview.
    pub provisional_schedule: Option<TimeRange>,
}

/// Dashboard layout state used only to demonstrate review interactions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MockLayout {
    /// Current canonical timeline span in the review grid.
    pub timeline_span: u8,
    /// Whether supporting widgets have their display order swapped.
    pub swapped_supporting_widgets: bool,
}

/// Local draft state for the layout-edit flow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LayoutEditState {
    /// Widgets receive their ordinary interactions.
    Viewing,
    /// A draft and its exact pre-edit document are held locally.
    Editing { original: MockLayout },
}

/// Local category-screen controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CategoryState {
    /// Search text is retained while navigating away.
    pub search: String,
    /// Whether the local list shows uncategorized applications only.
    pub uncategorized_only: bool,
    /// Selected row indexes for bulk assignment.
    pub selected_rows: BTreeSet<usize>,
    /// Last local assignment acknowledgement.
    pub assigned_category: Option<String>,
}

/// Local settings disclosure state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SettingsState {
    /// Whether technical/advanced settings are visible.
    pub advanced_visible: bool,
}

/// A visible command reconciliation state with no external side effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CommandState {
    /// No mock command is awaiting feedback.
    Idle,
    /// The UI acknowledged an action before its simulated response.
    Pending(&'static str),
    /// The local test harness accepted the action.
    Confirmed(&'static str),
    /// The local test harness rejected the action.
    Rejected(&'static str),
}

/// All mutable state held by the review-only spike.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpikeState {
    /// Active destination.
    pub route: Route,
    /// Selected display-state fixture.
    pub data_state: MockDataState,
    /// Fixed logical-width preview.
    pub preview_width: PreviewWidth,
    /// Active distribution alternative.
    pub distribution: DistributionPresentation,
    /// Immutable mock data.
    pub snapshot: MockSnapshot,
    /// Today date offset relative to the current day.
    pub today_offset: i8,
    /// Current timeline interaction state.
    pub timeline: TimelineState,
    /// Whether one application filter is active.
    pub application_filter: bool,
    /// Whether one category filter is active.
    pub category_filter: bool,
    /// Current layout document.
    pub layout: MockLayout,
    /// Layout edit lifecycle.
    pub layout_edit: LayoutEditState,
    /// Current Overview range.
    pub overview_range: OverviewRange,
    /// Local saved-view acknowledgement.
    pub saved_view_name: Option<String>,
    /// Category interaction state.
    pub categories: CategoryState,
    /// Calendar date offset relative to today.
    pub calendar_offset: i8,
    /// Settings progressive disclosure state.
    pub settings: SettingsState,
    /// Visible local command reconciliation status.
    pub command: CommandState,
}

impl Default for SpikeState {
    fn default() -> Self {
        Self {
            route: Route::Today,
            data_state: MockDataState::Ready,
            preview_width: PreviewWidth::Standard,
            distribution: DistributionPresentation::StackedBar,
            snapshot: MockSnapshot::deterministic(),
            today_offset: 0,
            timeline: TimelineState {
                view_start: 0.0,
                view_span: TIMELINE_DURATION,
                hover: None,
                selection: None,
                drag_origin: None,
                create_schedule_mode: false,
                provisional_schedule: None,
            },
            application_filter: false,
            category_filter: false,
            layout: MockLayout {
                timeline_span: 12,
                swapped_supporting_widgets: false,
            },
            layout_edit: LayoutEditState::Viewing,
            overview_range: OverviewRange::Week,
            saved_view_name: None,
            categories: CategoryState {
                search: String::new(),
                uncategorized_only: false,
                selected_rows: BTreeSet::new(),
                assigned_category: None,
            },
            calendar_offset: 0,
            settings: SettingsState {
                advanced_visible: false,
            },
            command: CommandState::Idle,
        }
    }
}

/// A local interaction accepted by the spike reducer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpikeAction {
    /// Select a primary destination.
    Navigate(Route),
    /// Select a deterministic data presentation state.
    SelectDataState(MockDataState),
    /// Select a logical-width preview.
    SelectPreviewWidth(PreviewWidth),
    /// Select a visual distribution alternative.
    SelectDistribution(DistributionPresentation),
    /// Move the Today view one earlier day.
    TodayPrevious,
    /// Move the Today view one later day when allowed.
    TodayNext,
    /// Return the Today view to the current day.
    TodayGoCurrent,
    /// Toggle a mock application filter.
    ToggleApplicationFilter,
    /// Toggle a mock category filter.
    ToggleCategoryFilter,
    /// Clear all narrowing criteria.
    ClearNarrowing,
    /// Update the transient one-target hover state.
    SetHover(Option<MockSegmentRef>),
    /// Select one timeline segment.
    SelectSegment(MockSegmentRef),
    /// Select a timeline range.
    SelectRange(TimeRange),
    /// Clear a persistent timeline selection.
    ClearTimelineSelection,
    /// Pan the shared timeline transform.
    PanTimeline(f32),
    /// Zoom the shared timeline transform by a multiplier.
    ZoomTimeline(f32),
    /// Restore the default timeline view.
    ResetTimeline,
    /// Begin an interaction-response drag.
    BeginTimelineDrag(f32),
    /// Finish an interaction-response drag.
    EndTimelineDrag(f32),
    /// Enter or leave the explicit schedule creation mode.
    ToggleCreateScheduleMode,
    /// Make a mock command visibly pending.
    BeginCommand(&'static str),
    /// Confirm the most recent mock command.
    ConfirmCommand(&'static str),
    /// Reject the most recent mock command.
    RejectCommand(&'static str),
    /// Enter explicit layout editing.
    BeginLayoutEdit,
    /// Swap supporting-widget ordering in the draft.
    SwapSupportingWidgets,
    /// Step the timeline width through valid mock spans.
    ResizeTimeline,
    /// Save the local layout draft.
    SaveLayout,
    /// Restore the document active on layout-edit entry.
    CancelLayout,
    /// Load the built-in layout as a draft.
    ResetLayout,
    /// Change an Overview date-range choice.
    SetOverviewRange(OverviewRange),
    /// Locally acknowledge a saved-view request.
    SaveOverviewView,
    /// Retain a Categories search query.
    SetCategorySearch(String),
    /// Toggle the uncategorized-only filter.
    ToggleUncategorizedOnly,
    /// Toggle a row in the local bulk selection.
    ToggleCategoryRow(usize),
    /// Acknowledge a local bulk category assignment.
    AssignSelectedCategory(String),
    /// Move Calendar one day backward or forward.
    ShiftCalendar(i8),
    /// Return Calendar to the current day.
    CalendarGoCurrent,
    /// Toggle advanced Settings disclosure.
    ToggleAdvancedSettings,
}

impl SpikeState {
    /// Applies a local action without issuing a production command or performing I/O.
    pub(crate) fn reduce(&mut self, action: SpikeAction) {
        match action {
            SpikeAction::Navigate(route) => self.route = route,
            SpikeAction::SelectDataState(data_state) => self.data_state = data_state,
            SpikeAction::SelectPreviewWidth(width) => self.preview_width = width,
            SpikeAction::SelectDistribution(presentation) => self.distribution = presentation,
            SpikeAction::TodayPrevious => self.change_today(-1),
            SpikeAction::TodayNext if self.today_offset < 0 => self.change_today(1),
            SpikeAction::TodayNext | SpikeAction::TodayGoCurrent => self.set_today_offset(0),
            SpikeAction::ToggleApplicationFilter => {
                self.application_filter = !self.application_filter
            }
            SpikeAction::ToggleCategoryFilter => self.category_filter = !self.category_filter,
            SpikeAction::ClearNarrowing => {
                self.application_filter = false;
                self.category_filter = false;
                self.timeline.selection = None;
            }
            SpikeAction::SetHover(hover) => self.timeline.hover = hover,
            SpikeAction::SelectSegment(segment) => {
                self.timeline.selection = Some(MockSelection::Segment(segment));
            }
            SpikeAction::SelectRange(range) if range.is_meaningful() => {
                self.timeline.selection = Some(MockSelection::Range(range));
            }
            SpikeAction::SelectRange(_) | SpikeAction::ClearTimelineSelection => {
                self.timeline.selection = None;
            }
            SpikeAction::PanTimeline(delta) => self.pan_timeline(delta),
            SpikeAction::ZoomTimeline(multiplier) => self.zoom_timeline(multiplier),
            SpikeAction::ResetTimeline => {
                self.timeline.view_start = 0.0;
                self.timeline.view_span = TIMELINE_DURATION;
                self.timeline.selection = None;
            }
            SpikeAction::BeginTimelineDrag(origin) => self.timeline.drag_origin = Some(origin),
            SpikeAction::EndTimelineDrag(end) => self.finish_timeline_drag(end),
            SpikeAction::ToggleCreateScheduleMode => {
                self.timeline.create_schedule_mode = !self.timeline.create_schedule_mode;
                self.timeline.drag_origin = None;
            }
            SpikeAction::BeginCommand(message) => self.command = CommandState::Pending(message),
            SpikeAction::ConfirmCommand(message) => self.command = CommandState::Confirmed(message),
            SpikeAction::RejectCommand(message) => self.command = CommandState::Rejected(message),
            SpikeAction::BeginLayoutEdit => self.begin_layout_edit(),
            SpikeAction::SwapSupportingWidgets => {
                if matches!(self.layout_edit, LayoutEditState::Editing { .. }) {
                    self.layout.swapped_supporting_widgets =
                        !self.layout.swapped_supporting_widgets;
                }
            }
            SpikeAction::ResizeTimeline => {
                if matches!(self.layout_edit, LayoutEditState::Editing { .. }) {
                    self.layout.timeline_span = if self.layout.timeline_span == 12 {
                        8
                    } else {
                        12
                    };
                }
            }
            SpikeAction::SaveLayout => {
                if matches!(self.layout_edit, LayoutEditState::Editing { .. }) {
                    self.layout_edit = LayoutEditState::Viewing;
                    self.command = CommandState::Confirmed("Layout saved locally");
                }
            }
            SpikeAction::CancelLayout => self.cancel_layout_edit(),
            SpikeAction::ResetLayout => {
                if matches!(self.layout_edit, LayoutEditState::Editing { .. }) {
                    self.layout = MockLayout {
                        timeline_span: 12,
                        swapped_supporting_widgets: false,
                    };
                }
            }
            SpikeAction::SetOverviewRange(range) => self.overview_range = range,
            SpikeAction::SaveOverviewView => {
                self.saved_view_name = Some(format!("{} review", self.overview_range.label()));
                self.command = CommandState::Confirmed("Overview view saved locally");
            }
            SpikeAction::SetCategorySearch(search) => self.categories.search = search,
            SpikeAction::ToggleUncategorizedOnly => {
                self.categories.uncategorized_only = !self.categories.uncategorized_only;
            }
            SpikeAction::ToggleCategoryRow(index) => {
                if !self.categories.selected_rows.insert(index) {
                    self.categories.selected_rows.remove(&index);
                }
            }
            SpikeAction::AssignSelectedCategory(category) => {
                self.categories.assigned_category = Some(category.clone());
                self.command = CommandState::Pending("Assigning selected applications");
            }
            SpikeAction::ShiftCalendar(delta) => {
                self.calendar_offset = self.calendar_offset.saturating_add(delta).min(0);
            }
            SpikeAction::CalendarGoCurrent => self.calendar_offset = 0,
            SpikeAction::ToggleAdvancedSettings => {
                self.settings.advanced_visible = !self.settings.advanced_visible;
            }
        }
    }

    /// Returns a concise plain-language description of the active filters and selection.
    #[must_use]
    pub(crate) fn narrowing_summary(&self) -> String {
        let mut entries = Vec::new();
        if self.application_filter {
            entries.push("Application: Application 1");
        }
        if self.category_filter {
            entries.push("Category: Productive");
        }
        if self.timeline.selection.is_some() {
            entries.push("Selected time");
        }
        if entries.is_empty() {
            "No active filters or selection".to_owned()
        } else {
            entries.join("  ·  ")
        }
    }

    fn change_today(&mut self, delta: i8) {
        self.set_today_offset(self.today_offset.saturating_add(delta).min(0));
    }

    fn set_today_offset(&mut self, offset: i8) {
        if self.today_offset != offset {
            self.today_offset = offset;
            self.timeline.selection = None;
        }
    }

    fn pan_timeline(&mut self, delta: f32) {
        let max_start = (TIMELINE_DURATION - self.timeline.view_span).max(0.0);
        self.timeline.view_start = (self.timeline.view_start + delta).clamp(0.0, max_start);
    }

    fn zoom_timeline(&mut self, multiplier: f32) {
        let old_span = self.timeline.view_span;
        let new_span = (old_span * multiplier).clamp(20.0, TIMELINE_DURATION);
        let center = self.timeline.view_start + old_span / 2.0;
        let max_start = (TIMELINE_DURATION - new_span).max(0.0);
        self.timeline.view_span = new_span;
        self.timeline.view_start = (center - new_span / 2.0).clamp(0.0, max_start);
    }

    fn finish_timeline_drag(&mut self, end: f32) {
        let Some(origin) = self.timeline.drag_origin.take() else {
            return;
        };
        let range = TimeRange::ordered(origin, end);
        if !range.is_meaningful() {
            return;
        }
        if self.timeline.create_schedule_mode {
            self.timeline.provisional_schedule = Some(range);
            self.command = CommandState::Pending("Schedule draft is ready to review");
        } else {
            self.reduce(SpikeAction::SelectRange(range));
        }
    }

    fn begin_layout_edit(&mut self) {
        if matches!(self.layout_edit, LayoutEditState::Viewing) {
            self.layout_edit = LayoutEditState::Editing {
                original: self.layout.clone(),
            };
        }
    }

    fn cancel_layout_edit(&mut self) {
        let LayoutEditState::Editing { original } = &self.layout_edit else {
            return;
        };
        self.layout = original.clone();
        self.layout_edit = LayoutEditState::Viewing;
        self.command = CommandState::Confirmed("Layout draft reverted locally");
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutEditState, SpikeAction, SpikeState, TimeRange};

    #[test]
    fn today_navigation_never_moves_past_the_current_day() {
        let mut state = SpikeState::default();
        state.reduce(SpikeAction::TodayNext);
        assert_eq!(state.today_offset, 0);
        state.reduce(SpikeAction::TodayPrevious);
        assert_eq!(state.today_offset, -1);
        state.reduce(SpikeAction::TodayNext);
        assert_eq!(state.today_offset, 0);
    }

    #[test]
    fn cancel_layout_restores_the_exact_entry_document() {
        let mut state = SpikeState::default();
        state.reduce(SpikeAction::BeginLayoutEdit);
        state.reduce(SpikeAction::ResizeTimeline);
        state.reduce(SpikeAction::SwapSupportingWidgets);
        state.reduce(SpikeAction::CancelLayout);
        assert_eq!(state.layout.timeline_span, 12);
        assert!(!state.layout.swapped_supporting_widgets);
        assert!(matches!(state.layout_edit, LayoutEditState::Viewing));
    }

    #[test]
    fn schedule_mode_turns_a_drag_into_a_pending_draft() {
        let mut state = SpikeState::default();
        state.reduce(SpikeAction::ToggleCreateScheduleMode);
        state.reduce(SpikeAction::BeginTimelineDrag(10.0));
        state.reduce(SpikeAction::EndTimelineDrag(45.0));
        assert_eq!(
            state.timeline.provisional_schedule,
            Some(TimeRange::ordered(10.0, 45.0))
        );
        assert!(state.timeline.selection.is_none());
    }
}
