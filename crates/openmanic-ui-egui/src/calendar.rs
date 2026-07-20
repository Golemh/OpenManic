//! Calendar-day presentation from immutable application snapshots.
//!
//! The Phase 4 Calendar renderer remains adapter-free. Composition owns snapshot delivery and
//! command dispatching while this module maps interactions to typed UI actions.

use std::sync::Arc;

use openmanic_application::{
    ActivityTimelineNavigation, CalendarBlock, CalendarBlockId, CalendarBlockSource,
    CalendarDaySnapshot,
};
use openmanic_domain::HalfOpenInterval;

use crate::{EmptyReason, PresentableData, UserFacingError};

/// A Calendar-local interaction.  Date offsets are relative to the current local day supplied
/// by composition, so this module neither reads a clock nor resolves a civil date.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarAction {
    /// Moves to the preceding local day.
    PreviousDay,
    /// Moves to the next local day when the selected day is historical.
    NextDay,
    /// Selects a day from the date picker. Future selections normalize to today.
    SelectDateOffset {
        /// Offset from the current local day.
        day_offset: i32,
    },
    /// Returns to the current local day in one action.
    ReturnToToday,
    /// Makes one source block's exact details persistent.
    SelectBlock {
        /// Stable identity of the inspected block.
        id: CalendarBlockId,
    },
    /// Selects a dense period and makes its earliest block scroll-ready.
    SelectDensePeriod {
        /// Index in the current deterministic dense-period list.
        index: usize,
    },
    /// Routes the currently selected recorded block to Timeline when it has one.
    NavigateSelectedActivityToTimeline,
}

/// An effect the shell must route after a pure Calendar interaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarEffect {
    /// The selected day changed and a correlated Calendar snapshot is needed.
    RequestDay {
        /// Offset from the current local day to project.
        day_offset: i32,
    },
    /// A recorded activity should open at its exact Timeline context.
    NavigateToTimeline(ActivityTimelineNavigation),
}

/// The deliberately distinct visual treatment for each authoritative source kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarBlockKind {
    /// Recorded application/tracking data, rendered as the base lane.
    Activity,
    /// A focus or break overlay.
    Focus,
    /// A personal schedule overlay/enclosure.
    Schedule,
}

/// A render-ready source block that retains the immutable application value for details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarPresentedBlock {
    block: CalendarBlock,
    kind: CalendarBlockKind,
    selected: bool,
}

impl CalendarPresentedBlock {
    /// Returns the authoritative source block, including canonical and clipped ranges.
    #[must_use]
    pub const fn block(&self) -> &CalendarBlock {
        &self.block
    }

    /// Returns the source-specific visual treatment; color is not the only distinction.
    #[must_use]
    pub const fn kind(&self) -> CalendarBlockKind {
        self.kind
    }

    /// Returns whether this block's exact source and time details are selected.
    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }
}

/// Exact source details for a selected block. The contained block preserves canonical truth,
/// visual clipping, and midnight continuation markers without a duplicated UI data model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarBlockDetails {
    block: CalendarBlock,
    kind: CalendarBlockKind,
}

impl CalendarBlockDetails {
    /// Returns the stable selected block identity.
    #[must_use]
    pub const fn id(&self) -> CalendarBlockId {
        self.block.id()
    }

    /// Returns the authoritative source-specific detail value.
    #[must_use]
    pub const fn source(&self) -> &CalendarBlockSource {
        self.block.source()
    }

    /// Returns the complete source interval, before day clipping.
    #[must_use]
    pub const fn canonical_range(&self) -> HalfOpenInterval {
        self.block.canonical_range()
    }

    /// Returns the exact selected-day contribution shown on the vertical axis.
    #[must_use]
    pub const fn visual_range(&self) -> HalfOpenInterval {
        self.block.visual_range()
    }

    /// Returns the distinct source treatment needed by the inspector.
    #[must_use]
    pub const fn kind(&self) -> CalendarBlockKind {
        self.kind
    }

    /// Returns Timeline navigation only for a recorded activity block.
    #[must_use]
    pub const fn activity_timeline_navigation(&self) -> Option<ActivityTimelineNavigation> {
        self.block.activity_timeline_navigation()
    }
}

/// A group of simultaneous or closely stacked blocks that a compact renderer can navigate to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarDensePeriod {
    range: HalfOpenInterval,
    block_ids: Vec<CalendarBlockId>,
    scroll_target: CalendarBlockId,
}

impl CalendarDensePeriod {
    /// Returns the full interval occupied by the dense group.
    #[must_use]
    pub const fn range(&self) -> HalfOpenInterval {
        self.range
    }

    /// Returns the stable member identities in deterministic source/time order.
    #[must_use]
    pub fn block_ids(&self) -> &[CalendarBlockId] {
        &self.block_ids
    }

    /// Returns the first selected block a renderer should reveal when this period is chosen.
    #[must_use]
    pub const fn scroll_target(&self) -> CalendarBlockId {
        self.scroll_target
    }
}

/// Explicit data state used by a Calendar renderer.
#[derive(Clone, Debug)]
pub enum CalendarDataState {
    /// No correlated snapshot is available yet.
    Loading,
    /// A successful response contained no Calendar blocks.
    Empty(EmptyReason),
    /// Usable data, optionally with a non-blocking presentation notice.
    Ready {
        /// Optional non-blocking presentation notice.
        notice: Option<String>,
    },
    /// No usable snapshot is available; retain ordinary-language recovery text.
    Error(UserFacingError),
}

/// Complete pure renderer input derived from one immutable Calendar snapshot.
#[derive(Clone, Debug)]
pub struct CalendarViewModel {
    selected_day_offset: i32,
    next_day_enabled: bool,
    state: CalendarDataState,
    blocks: Vec<CalendarPresentedBlock>,
    selected_details: Option<CalendarBlockDetails>,
    dense_periods: Vec<CalendarDensePeriod>,
    selected_dense_period: Option<usize>,
}

impl CalendarViewModel {
    /// Returns the date-picker offset from the current local day.
    #[must_use]
    pub const fn selected_day_offset(&self) -> i32 {
        self.selected_day_offset
    }

    /// Returns whether Next day is valid; Calendar never navigates into the future.
    #[must_use]
    pub const fn next_day_enabled(&self) -> bool {
        self.next_day_enabled
    }

    /// Returns loading, empty, usable, or error state without querying any port.
    #[must_use]
    pub const fn state(&self) -> &CalendarDataState {
        &self.state
    }

    /// Returns all source-distinct blocks in deterministic display order.
    #[must_use]
    pub fn blocks(&self) -> &[CalendarPresentedBlock] {
        &self.blocks
    }

    /// Returns exact selected source/time details, when a selected block remains visible.
    #[must_use]
    pub const fn selected_details(&self) -> Option<&CalendarBlockDetails> {
        self.selected_details.as_ref()
    }

    /// Returns compact-navigation groups for periods with multiple simultaneously visible blocks.
    #[must_use]
    pub fn dense_periods(&self) -> &[CalendarDensePeriod] {
        &self.dense_periods
    }

    /// Returns the selected dense-period index, if any.
    #[must_use]
    pub const fn selected_dense_period(&self) -> Option<usize> {
        self.selected_dense_period
    }
}

/// UI-local Calendar state and action reducer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CalendarController {
    day_offset: i32,
    block: Option<CalendarBlockId>,
    dense_period: Option<usize>,
}

impl CalendarController {
    /// Reduces one action and returns any shell-routed effect.
    #[must_use]
    pub fn apply(
        &mut self,
        action: CalendarAction,
        snapshot: Option<&CalendarDaySnapshot>,
    ) -> Option<CalendarEffect> {
        match action {
            CalendarAction::PreviousDay => self.set_day_offset(self.day_offset.saturating_sub(1)),
            CalendarAction::NextDay if self.day_offset < 0 => {
                self.set_day_offset(self.day_offset.saturating_add(1))
            }
            CalendarAction::NextDay => None,
            CalendarAction::SelectDateOffset { day_offset } => self.set_day_offset(day_offset),
            CalendarAction::ReturnToToday => self.set_day_offset(0),
            CalendarAction::SelectBlock { id } => {
                self.block = snapshot.and_then(|value| find_block(value, id).map(|_| id));
                self.dense_period = None;
                None
            }
            CalendarAction::SelectDensePeriod { index } => {
                let value = snapshot?;
                let periods = dense_periods(value);
                let period = periods.get(index)?;
                self.dense_period = Some(index);
                self.block = Some(period.scroll_target());
                None
            }
            CalendarAction::NavigateSelectedActivityToTimeline => snapshot
                .and_then(|value| self.block.and_then(|id| find_block(value, id)))
                .and_then(CalendarBlock::activity_timeline_navigation)
                .map(CalendarEffect::NavigateToTimeline),
        }
    }

    /// Derives all presentation state from immutable snapshot data.
    #[must_use]
    pub fn view_model(&self, data: &PresentableData<CalendarDaySnapshot>) -> CalendarViewModel {
        let (state, snapshot) = calendar_state(data);
        let (blocks, details, periods) = snapshot.map_or_else(
            || (Vec::new(), None, Vec::new()),
            |value| {
                let blocks = presented_blocks(value, self.block);
                let details = self
                    .block
                    .and_then(|id| find_block(value, id))
                    .map(block_details);
                let periods = dense_periods(value);
                (blocks, details, periods)
            },
        );
        let selected_dense_period = self.dense_period.filter(|index| *index < periods.len());
        CalendarViewModel {
            selected_day_offset: self.day_offset,
            next_day_enabled: self.day_offset < 0,
            state,
            blocks,
            selected_details: details,
            dense_periods: periods,
            selected_dense_period,
        }
    }

    fn set_day_offset(&mut self, requested: i32) -> Option<CalendarEffect> {
        let normalized = requested.min(0);
        if normalized == self.day_offset {
            return None;
        }
        self.day_offset = normalized;
        self.block = None;
        self.dense_period = None;
        Some(CalendarEffect::RequestDay {
            day_offset: normalized,
        })
    }
}

fn calendar_state(
    data: &PresentableData<CalendarDaySnapshot>,
) -> (CalendarDataState, Option<&CalendarDaySnapshot>) {
    match data {
        PresentableData::InitialLoading => (CalendarDataState::Loading, None),
        PresentableData::Empty(reason) => (CalendarDataState::Empty(*reason), None),
        PresentableData::Failed { prior: None, error } => {
            (CalendarDataState::Error(error.clone()), None)
        }
        PresentableData::Ready(value) => ready_state(value, None),
        PresentableData::Partial { value, .. } => {
            ready_state(value, Some("Calendar data is partial."))
        }
        PresentableData::Refreshing { prior, .. } => {
            ready_state(prior, Some("Refreshing calendar data."))
        }
        PresentableData::Recovered { value, notice } => ready_state(value, Some(notice.as_str())),
        PresentableData::Failed {
            prior: Some(value), ..
        } => ready_state(
            value,
            Some("Calendar data could not be refreshed. Showing the last available data."),
        ),
    }
}

fn ready_state<'a>(
    snapshot: &'a Arc<CalendarDaySnapshot>,
    notice: Option<&str>,
) -> (CalendarDataState, Option<&'a CalendarDaySnapshot>) {
    let has_blocks = !snapshot.activity_blocks().is_empty()
        || !snapshot.focus_blocks().is_empty()
        || !snapshot.schedule_blocks().is_empty();
    let state = if has_blocks {
        CalendarDataState::Ready {
            notice: notice.map(str::to_owned),
        }
    } else {
        CalendarDataState::Empty(EmptyReason::NoRecordedActivity)
    };
    (state, Some(Arc::as_ref(snapshot)))
}

fn presented_blocks(
    snapshot: &CalendarDaySnapshot,
    selected: Option<CalendarBlockId>,
) -> Vec<CalendarPresentedBlock> {
    let mut values = Vec::with_capacity(
        snapshot.activity_blocks().len()
            + snapshot.focus_blocks().len()
            + snapshot.schedule_blocks().len(),
    );
    append_blocks(
        &mut values,
        snapshot.activity_blocks(),
        CalendarBlockKind::Activity,
        selected,
    );
    append_blocks(
        &mut values,
        snapshot.focus_blocks(),
        CalendarBlockKind::Focus,
        selected,
    );
    append_blocks(
        &mut values,
        snapshot.schedule_blocks(),
        CalendarBlockKind::Schedule,
        selected,
    );
    values.sort_by_key(|value| value.block.visual_range().start());
    values
}

fn append_blocks(
    output: &mut Vec<CalendarPresentedBlock>,
    blocks: &[CalendarBlock],
    kind: CalendarBlockKind,
    selected: Option<CalendarBlockId>,
) {
    output.extend(blocks.iter().cloned().map(|block| CalendarPresentedBlock {
        selected: Some(block.id()) == selected,
        block,
        kind,
    }));
}

fn block_details(block: &CalendarBlock) -> CalendarBlockDetails {
    CalendarBlockDetails {
        kind: block_kind(block),
        block: block.clone(),
    }
}

const fn block_kind(block: &CalendarBlock) -> CalendarBlockKind {
    match block.source() {
        CalendarBlockSource::Activity { .. } => CalendarBlockKind::Activity,
        CalendarBlockSource::Focus { .. } => CalendarBlockKind::Focus,
        CalendarBlockSource::Schedule { .. } => CalendarBlockKind::Schedule,
    }
}

fn find_block(snapshot: &CalendarDaySnapshot, id: CalendarBlockId) -> Option<&CalendarBlock> {
    snapshot
        .activity_blocks()
        .iter()
        .chain(snapshot.focus_blocks())
        .chain(snapshot.schedule_blocks())
        .find(|block| block.id() == id)
}

fn dense_periods(snapshot: &CalendarDaySnapshot) -> Vec<CalendarDensePeriod> {
    let mut blocks: Vec<&CalendarBlock> = snapshot
        .activity_blocks()
        .iter()
        .chain(snapshot.focus_blocks())
        .chain(snapshot.schedule_blocks())
        .collect();
    blocks.sort_by_key(|block| block.visual_range().start());
    let mut periods = Vec::new();
    let mut group: Vec<&CalendarBlock> = Vec::new();
    let mut group_end = None;
    for block in blocks {
        if group_end.is_some_and(|end| block.visual_range().start() >= end) {
            push_dense_period(&mut periods, &group);
            group.clear();
            group_end = None;
        }
        group_end = Some(group_end.map_or(block.visual_range().end(), |end| {
            end.max(block.visual_range().end())
        }));
        group.push(block);
    }
    push_dense_period(&mut periods, &group);
    periods
}

fn push_dense_period(periods: &mut Vec<CalendarDensePeriod>, blocks: &[&CalendarBlock]) {
    if blocks.len() < 2 {
        return;
    }
    let start = blocks[0].visual_range().start();
    let end = blocks
        .iter()
        .map(|block| block.visual_range().end())
        .max()
        .unwrap_or(start);
    if let Ok(range) = HalfOpenInterval::try_new(start, end) {
        periods.push(CalendarDensePeriod {
            range,
            block_ids: blocks.iter().map(|block| block.id()).collect(),
            scroll_target: blocks[0].id(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openmanic_application::{
        CalendarDayContext, CalendarDayProjector, CalendarProjectionSource, CalendarSourceFocus,
        DataRevision, FocusKind, FocusSnapshot, ProjectionContextKey, TimelineRawIntervalId,
        TimelineSourceActivity,
    };
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, FocusSessionId,
        FocusSessionState, HalfOpenInterval, TrackerRunId, UtcMicros,
    };

    use super::{
        CalendarAction, CalendarBlockKind, CalendarController, CalendarDataState, CalendarEffect,
    };
    use crate::PresentableData;

    #[test]
    fn date_controls_normalize_future_days_and_disable_next_at_today() {
        let mut controller = CalendarController::default();
        assert_eq!(
            controller.apply(CalendarAction::NextDay, None),
            None,
            "Next cannot enter a future day"
        );
        assert_eq!(
            controller.apply(CalendarAction::PreviousDay, None),
            Some(CalendarEffect::RequestDay { day_offset: -1 })
        );
        assert_eq!(
            controller.apply(CalendarAction::SelectDateOffset { day_offset: 4 }, None),
            Some(CalendarEffect::RequestDay { day_offset: 0 })
        );
        let model = controller.view_model(&PresentableData::InitialLoading);
        assert_eq!(model.selected_day_offset(), 0);
        assert!(!model.next_day_enabled());
        assert!(matches!(model.state(), CalendarDataState::Loading));
    }

    #[test]
    fn selected_activity_retains_exact_ranges_and_routes_to_timeline() {
        let snapshot = activity_snapshot();
        let activity = &snapshot.activity_blocks()[0];
        let mut controller = CalendarController::default();
        let _ = controller.apply(
            CalendarAction::SelectBlock { id: activity.id() },
            Some(&snapshot),
        );

        let model = controller.view_model(&PresentableData::Ready(Arc::new(snapshot.clone())));
        let details = model
            .selected_details()
            .expect("selected activity remains inspectable");
        assert_eq!(details.kind(), CalendarBlockKind::Activity);
        assert_eq!(details.canonical_range(), range(90, 120));
        assert_eq!(details.visual_range(), range(100, 120));
        assert_eq!(
            controller.apply(
                CalendarAction::NavigateSelectedActivityToTimeline,
                Some(&snapshot)
            ),
            Some(CalendarEffect::NavigateToTimeline(
                activity
                    .activity_timeline_navigation()
                    .expect("recorded activity is navigable")
            ))
        );
    }

    #[test]
    fn overlapping_sources_form_a_scroll_ready_dense_period() {
        let snapshot = activity_and_focus_snapshot();
        let mut controller = CalendarController::default();
        let model = controller.view_model(&PresentableData::Ready(Arc::new(snapshot.clone())));
        assert_eq!(model.blocks().len(), 2);
        assert_eq!(model.blocks()[0].kind(), CalendarBlockKind::Activity);
        assert_eq!(model.blocks()[1].kind(), CalendarBlockKind::Focus);
        assert_eq!(model.dense_periods().len(), 1);
        let target = model.dense_periods()[0].scroll_target();

        let _ = controller.apply(
            CalendarAction::SelectDensePeriod { index: 0 },
            Some(&snapshot),
        );
        let view_model = controller.view_model(&PresentableData::Ready(Arc::new(snapshot)));
        let selected = view_model
            .selected_details()
            .expect("dense period selects its scroll target");
        assert_eq!(selected.id(), target);
    }

    fn activity_snapshot() -> openmanic_application::CalendarDaySnapshot {
        CalendarDayProjector::build(
            context(),
            CalendarProjectionSource::new(DataRevision::new(1), &[activity(9, 90, 120)], &[], &[]),
        )
        .expect("valid activity projects")
    }

    fn activity_and_focus_snapshot() -> openmanic_application::CalendarDaySnapshot {
        let focus = FocusSnapshot::try_restore(
            FocusSessionId::from_bytes([3; 16]),
            FocusKind::Focus,
            Some("Write".to_owned()),
            30,
            None,
            FocusSessionState::Completed {
                started_at: UtcMicros::new(110),
                completed_at: UtcMicros::new(140),
            },
            openmanic_application::EntityRevision::new(0),
        )
        .expect("valid completed focus");
        CalendarDayProjector::build(
            context(),
            CalendarProjectionSource::new(
                DataRevision::new(1),
                &[activity(9, 100, 150)],
                &[CalendarSourceFocus::new(focus, range(110, 140))],
                &[],
            ),
        )
        .expect("overlays are retained")
    }

    fn context() -> CalendarDayContext {
        CalendarDayContext::new(ProjectionContextKey::new(1), range(100, 200))
    }

    fn activity(raw_id: u64, start: i64, end: i64) -> TimelineSourceActivity {
        TimelineSourceActivity::new(
            TimelineRawIntervalId::new(raw_id),
            ActivityInterval::try_new(
                TrackerRunId::from_bytes([1; 16]),
                range(start, end),
                ActivityState::Active,
                ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
                    .expect("foreground evidence is valid"),
                Some(openmanic_domain::ApplicationId::from_bytes([2; 16])),
            )
            .expect("activity fixture is valid"),
        )
    }

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("range fixture is positive")
    }
}
