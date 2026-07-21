//! Paint-only three-band timeline rendering and UI-local detail state.
//!
//! The renderer consumes an immutable [`TimelineSnapshot`] and one [`PresentableData`] state. It
//! makes exactly one egui interaction allocation for the timeline canvas, draws interval geometry
//! through the OM-282 paint plan, and returns actions for its owner to reduce. It does not access
//! ports, clone history, or mutate canonical category/application data.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Key, Painter, PointerButton, Pos2, Rect, Response,
    Sense, Stroke, Ui, Vec2,
};
use openmanic_application::{
    ActivityStateValue, ApplicationBandValue, CategoryBandValue, DataCompleteness, TimelineSnapshot,
};
use openmanic_domain::{ActivityState, ApplicationId, CategoryId, HalfOpenInterval, UtcMicros};
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    PaintFill, PaintPrimitive, TimelineBand, TimelineDetail, TimelineDetailValue, TimelineGesture,
    TimelineGestureEvent, TimelineInteraction, TimelinePaintPlan, TimelineTransform, hit_test,
    prepare_schedule_overlays,
};
use crate::{DataLimitation, PresentableData, ThemeTokens, TodayAction, TodayViewContext};

const BAND_LABEL_WIDTH: f32 = 96.0;
const OVERVIEW_TOP_PADDING: f32 = 22.0;
const OVERVIEW_HEIGHT: f32 = 24.0;
const OVERVIEW_AXIS_GAP: f32 = 8.0;
const TICK_HEIGHT: f32 = 24.0;
const CATEGORY_BAND_HEIGHT: f32 = 112.0;
const ACTIVITY_BAND_HEIGHT: f32 = 16.0;
const APPLICATION_BAND_HEIGHT: f32 = 16.0;
const BAND_GAP: f32 = 8.0;
const TIMELINE_BOTTOM_PADDING: f32 = 6.0;
const TIMELINE_HEIGHT: f32 = OVERVIEW_TOP_PADDING
    + OVERVIEW_HEIGHT
    + OVERVIEW_AXIS_GAP
    + TICK_HEIGHT
    + CATEGORY_BAND_HEIGHT
    + BAND_GAP
    + ACTIVITY_BAND_HEIGHT
    + BAND_GAP
    + APPLICATION_BAND_HEIGHT
    + TIMELINE_BOTTOM_PADDING;
const OVERVIEW_EDGE_GRAB_WIDTH: f32 = 7.0;
const HOUR_US: i64 = 60 * 60 * 1_000_000;
const INITIAL_VIEW_HOURS: u64 = 4;
const INITIAL_VIEW_TRAILING_HOURS: u64 = 1;
const MIN_NAVIGATOR_RANGE_US: u64 = 15 * 60 * 1_000_000;
const DETAIL_WIDTH: f32 = 320.0;
const DETAIL_LINE_HEIGHT: f32 = 17.0;
const CATEGORY_LANE: usize = 0;
const ACTIVE_STATE: usize = 1;
const AWAY_STATE: usize = 2;
const APPLICATION_LANE: usize = 3;
const POWERED_OFF_STATE: usize = 4;

const TRACK: Color32 = crate::design::INSET;
const SUBWIDGET: Color32 = crate::design::BG_DEEP;
const BAND_BORDER: Color32 = crate::design::BORDER;
const VIEWFINDER_ACCENT: Color32 = crate::design::ACCENT;
const MUTED: Color32 = crate::design::TEXT_MUTED;
const SELECTED: Color32 = Color32::from_rgb(255, 255, 255);
const ERROR: Color32 = Color32::from_rgb(244, 63, 94);
const WARNING: Color32 = Color32::from_rgb(245, 158, 11);
const SUCCESS: Color32 = Color32::from_rgb(52, 211, 153);

/// An action emitted by the timeline renderer for its owning controller to reduce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineRenderAction {
    /// Applies a UI-local Today action after the frame; selection itself emits no mutation action.
    Today(TodayAction),
    /// Reports a UI-local pan or zoom range so the owner can schedule the next paint.
    ViewRangeChanged {
        /// The exact UTC range requested by the gesture kernel.
        range: HalfOpenInterval,
    },
    /// Requests the shared schedule editor for a non-mutating provisional range.
    ScheduleRequested {
        /// Exact provisional schedule boundaries.
        range: HalfOpenInterval,
    },
    /// Opens the selected application's category and privacy controls.
    OpenCategories {
        /// Stable application selected from an immutable timeline detail.
        application_id: ApplicationId,
    },
}

/// The renderer's bounded result for one normal egui frame.
#[derive(Clone, Debug, Default)]
pub struct TimelineRenderOutput {
    actions: Vec<TimelineRenderAction>,
    hover_detail: Option<TimelineDetail>,
    persistent_detail: Option<TimelineDetail>,
}

impl TimelineRenderOutput {
    /// Returns actions to be reduced by the caller after rendering.
    #[must_use]
    pub fn actions(&self) -> &[TimelineRenderAction] {
        &self.actions
    }

    /// Returns the sole transient pointer-adjacent detail, if the pointer hit one band.
    #[must_use]
    pub const fn hover_detail(&self) -> Option<TimelineDetail> {
        self.hover_detail
    }

    /// Returns the persistent detail selected by a click, if any.
    #[must_use]
    pub const fn persistent_detail(&self) -> Option<TimelineDetail> {
        self.persistent_detail
    }
}

/// UI-local retained state for one three-band timeline widget instance.
#[derive(Debug)]
pub struct TimelineRenderer {
    interaction: Option<TimelineInteraction>,
    persistent_detail: Option<TimelineDetail>,
    primary_down: bool,
    middle_down: bool,
    pressed_band: Option<TimelineBand>,
    last_pointer: Option<Pos2>,
    snapshot_range: Option<HalfOpenInterval>,
    default_view_range: Option<HalfOpenInterval>,
    session_opened_at: UtcMicros,
    auto_follow_default: bool,
    overview_capture: Option<OverviewCapture>,
    category_labels: BTreeMap<CategoryId, String>,
    application_labels: BTreeMap<ApplicationId, String>,
    lane_visibility: [bool; 5],
    theme_tokens: ThemeTokens,
}

impl Default for TimelineRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineRenderer {
    /// Creates an empty renderer that initializes itself from its first immutable snapshot.
    #[must_use]
    pub fn new() -> Self {
        let session_opened_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
            });
        Self::new_at(UtcMicros::new(session_opened_at))
    }

    /// Creates a renderer with an explicit session-start instant.
    ///
    /// This is primarily useful for deterministic tests and embedded hosts that already own a
    /// clock. The instant anchors today's initial viewport when no tracked interval exists yet.
    #[must_use]
    pub fn new_at(session_opened_at: UtcMicros) -> Self {
        Self {
            interaction: None,
            persistent_detail: None,
            primary_down: false,
            middle_down: false,
            pressed_band: None,
            last_pointer: None,
            snapshot_range: None,
            default_view_range: None,
            session_opened_at,
            auto_follow_default: true,
            overview_capture: None,
            category_labels: BTreeMap::new(),
            application_labels: BTreeMap::new(),
            lane_visibility: [true; 5],
            theme_tokens: ThemeTokens::default(),
        }
    }

    /// Replaces the complete semantic palette used by custom timeline paint.
    pub fn set_theme_tokens(&mut self, theme_tokens: ThemeTokens) {
        self.theme_tokens = theme_tokens;
    }

    /// Replaces the display labels used inside sufficiently wide category segments.
    pub fn set_category_labels(&mut self, labels: impl IntoIterator<Item = (CategoryId, String)>) {
        self.category_labels = labels.into_iter().collect();
    }

    /// Replaces the display labels used inside sufficiently wide application segments.
    pub fn set_application_labels(
        &mut self,
        labels: impl IntoIterator<Item = (ApplicationId, String)>,
    ) {
        self.application_labels = labels.into_iter().collect();
    }

    /// Restores the selected day's adaptive initial viewport.
    ///
    /// Returns `true` when a zoomed or panned view changed and needs repainting.
    pub fn reset_view(&mut self) -> bool {
        let Some(default_range) = self.default_view_range else {
            return false;
        };
        let changed = !self.auto_follow_default
            || self
                .interaction
                .is_some_and(|interaction| interaction.view_range() != default_range);
        if changed {
            self.interaction = Some(TimelineInteraction::new(default_range));
            self.overview_capture = None;
            self.primary_down = false;
            self.middle_down = false;
            self.auto_follow_default = true;
        }
        changed
    }

    /// Renders one frame from presentation data without querying a port or mutating data.
    ///
    /// The supplied [`TodayViewContext`] is read only. Its existing range selection is painted,
    /// while this method returns deferred [`TimelineRenderAction`] values for the caller to apply.
    #[must_use]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        data: &PresentableData<TimelineSnapshot>,
        context: &TodayViewContext,
        create_schedule_mode: bool,
    ) -> TimelineRenderOutput {
        render_presentation_notice(ui, data);
        let Some(snapshot) = data.visible_value() else {
            render_unavailable_canvas(ui, unavailable_message(data));
            return TimelineRenderOutput {
                persistent_detail: self.persistent_detail,
                ..TimelineRenderOutput::default()
            };
        };
        self.show_snapshot(ui, snapshot, context, create_schedule_mode)
    }

    /// Renders an already-selected immutable snapshot.
    ///
    /// A composition root that owns a correlated multi-widget snapshot can use this entry point
    /// after presenting its own loading or error state, without rebuilding or querying data on
    /// the egui thread.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the renderer keeps one allocation, interaction pass, and deferred output together"
    )]
    pub fn show_snapshot(
        &mut self,
        ui: &mut Ui,
        snapshot: &TimelineSnapshot,
        context: &TodayViewContext,
        create_schedule_mode: bool,
    ) -> TimelineRenderOutput {
        self.ensure_interaction(snapshot);
        let (canvas, response) = ui.allocate_exact_size(
            Vec2::new(
                ui.available_width().max(BAND_LABEL_WIDTH + 1.0),
                TIMELINE_HEIGHT,
            ),
            Sense::click_and_drag(),
        );
        let chart_left = canvas.min.x + BAND_LABEL_WIDTH;
        let overview_rect = Rect::from_min_size(
            Pos2::new(chart_left, canvas.min.y + OVERVIEW_TOP_PADDING),
            Vec2::new(canvas.max.x - chart_left, OVERVIEW_HEIGHT),
        );
        let axis_top = overview_rect.max.y + OVERVIEW_AXIS_GAP;
        let timeline_rect = Rect::from_min_max(
            Pos2::new(chart_left, axis_top + TICK_HEIGHT),
            Pos2::new(canvas.max.x, canvas.max.y - TIMELINE_BOTTOM_PADDING),
        );
        let view_range = self
            .interaction
            .as_ref()
            .map_or(snapshot.visible_range(), |interaction| {
                interaction.view_range()
            });
        let Ok(transform) =
            TimelineTransform::try_new(view_range, timeline_rect.min.x, timeline_rect.width())
        else {
            return TimelineRenderOutput::default();
        };
        let band_rects = BandRects::new(timeline_rect);
        let overview_hover = response.hover_pos().and_then(|pointer| {
            overview_hover_at_pointer(pointer, overview_rect, snapshot.visible_range(), view_range)
        });
        let cursor = self
            .overview_capture
            .map(OverviewCapture::cursor_icon)
            .or_else(|| overview_hover.map(OverviewHover::cursor_icon));
        if let Some(cursor) = cursor {
            ui.ctx().set_cursor_icon(cursor);
        }
        self.paint_snapshot(
            ui.painter(),
            snapshot,
            transform,
            TimelinePaintLayout {
                canvas,
                overview: overview_rect,
                overview_hover,
                bands: band_rects,
            },
            context,
        );

        let mut output = TimelineRenderOutput::default();
        if let Some(range) = self.next_overview_range(
            ui,
            &response,
            overview_rect,
            snapshot.visible_range(),
            view_range,
        ) {
            self.apply_gesture(
                snapshot,
                transform,
                TimelineGesture::NavigateTo { range },
                &mut output,
            );
        } else if self.overview_capture.is_none()
            && let Some(gesture) =
                self.next_gesture(ui, &response, band_rects, create_schedule_mode)
        {
            self.apply_gesture(snapshot, transform, gesture, &mut output);
        }
        if !self.primary_down && !self.middle_down {
            output.hover_detail = self.hover_detail(snapshot, transform, band_rects);
        }
        let context_detail = response.hover_pos().and_then(|pointer| {
            let band = band_rects.band_at(pointer)?;
            detail_at_pointer(snapshot, transform, band, pointer.x)
        });
        response.context_menu(|ui| {
            ui.strong("Timeline options");
            ui.separator();
            let Some(detail) = context_detail else {
                ui.colored_label(self.theme_tokens.content_secondary(), "No segment here");
                return;
            };
            if ui.button("Inspect segment details").clicked() {
                self.persistent_detail = Some(detail);
                ui.close();
            }
            if let (Some(label), Some(action)) =
                (detail.value().action_label(), detail.value().action())
                && ui.button(label).clicked()
            {
                output.actions.push(TimelineRenderAction::Today(action));
                ui.close();
            }
            if let TimelineDetailValue::Application(ApplicationBandValue::Application(
                application_id,
            )) = detail.value()
                && ui.button("Edit application category and color").clicked()
            {
                output
                    .actions
                    .push(TimelineRenderAction::OpenCategories { application_id });
                ui.close();
            }
        });
        if let (Some(detail), Some(pointer)) = (output.hover_detail, self.last_pointer) {
            let label = self.detail_value_label(detail);
            paint_hover_detail(
                ui.painter(),
                ui.clip_rect(),
                pointer,
                detail,
                &label,
                snapshot.visible_range(),
                self.theme_tokens,
            );
        }
        render_timeline_legend(ui, snapshot, self.theme_tokens, &mut self.lane_visibility);
        if self.persistent_detail.is_none() {
            ui.colored_label(
                self.theme_tokens.content_secondary(),
                "Click a timeline segment for details, or drag to select a time range.",
            );
        }
        self.paint_persistent_detail(ui, &mut output, snapshot.visible_range());
        output.persistent_detail = self.persistent_detail;
        output
    }

    fn ensure_interaction(&mut self, snapshot: &TimelineSnapshot) {
        let day_range = snapshot.visible_range();
        let default_range = initial_view_range(snapshot, self.session_opened_at);
        let day_changed = self.snapshot_range != Some(day_range);
        let default_changed_while_following = self.auto_follow_default
            && self
                .interaction
                .as_ref()
                .is_some_and(|interaction| interaction.view_range() != default_range);
        if day_changed || self.interaction.is_none() || default_changed_while_following {
            self.interaction = Some(TimelineInteraction::new(default_range));
            self.snapshot_range = Some(day_range);
            self.default_view_range = Some(default_range);
            self.auto_follow_default = true;
        } else {
            self.default_view_range = Some(default_range);
        }
    }

    fn next_overview_range(
        &mut self,
        ui: &Ui,
        response: &Response,
        overview_rect: Rect,
        day_range: HalfOpenInterval,
        view_range: HalfOpenInterval,
    ) -> Option<HalfOpenInterval> {
        let input = ui.input(TimelineInput::from_input);
        if input.keys.escape_pressed {
            self.overview_capture = None;
            return None;
        }
        if !input.buttons.primary_down {
            self.overview_capture = None;
            return None;
        }
        let pointer = input.pointer?;
        if self.overview_capture.is_some() || overview_rect.contains(pointer) {
            self.last_pointer = Some(pointer);
        }
        if self.overview_capture.is_none() {
            if !response.hovered() || !overview_rect.contains(pointer) {
                return None;
            }
            let viewport = overview_view_rect(overview_rect, day_range, view_range)?;
            if pointer.x < viewport.left() - OVERVIEW_EDGE_GRAB_WIDTH
                || pointer.x > viewport.right() + OVERVIEW_EDGE_GRAB_WIDTH
            {
                return None;
            }
            let pointer_instant = overview_instant_at_x(overview_rect, day_range, pointer.x)?;
            self.overview_capture = Some(overview_capture_at_pointer(
                pointer,
                pointer_instant,
                viewport,
                view_range,
            ));
        }
        overview_range_for_pointer(overview_rect, day_range, self.overview_capture?, pointer.x)
    }

    fn next_gesture(
        &mut self,
        ui: &Ui,
        response: &Response,
        band_rects: BandRects,
        create_schedule_mode: bool,
    ) -> Option<TimelineGesture> {
        let input = ui.input(TimelineInput::from_input);
        let previous_pointer = self.last_pointer;
        self.last_pointer = input.pointer.or(previous_pointer);
        let pointer_over_bands = input
            .pointer
            .is_some_and(|pointer| band_rects.contains(pointer));
        if input.keys.escape_pressed {
            self.primary_down = false;
            self.middle_down = false;
            self.pressed_band = None;
            return Some(TimelineGesture::Escape);
        }
        if input.buttons.primary_down
            && !self.primary_down
            && response.hovered()
            && pointer_over_bands
        {
            self.primary_down = true;
            self.pressed_band = input
                .pointer
                .and_then(|pointer| band_rects.band_at(pointer));
            return input
                .pointer
                .map(|pointer| TimelineGesture::PrimaryPressed {
                    x: pointer.x,
                    space_held: input.keys.space_held,
                    create_schedule_mode,
                });
        }
        if !input.buttons.primary_down && self.primary_down {
            self.primary_down = false;
            return self
                .last_pointer
                .map(|pointer| TimelineGesture::PrimaryReleased { x: pointer.x });
        }
        if input.buttons.middle_down
            && !self.middle_down
            && response.hovered()
            && pointer_over_bands
        {
            self.middle_down = true;
            return input
                .pointer
                .map(|pointer| TimelineGesture::MiddlePressed { x: pointer.x });
        }
        if !input.buttons.middle_down && self.middle_down {
            self.middle_down = false;
            return self
                .last_pointer
                .map(|pointer| TimelineGesture::MiddleReleased { x: pointer.x });
        }
        if (self.primary_down || self.middle_down) && input.pointer != previous_pointer {
            return input
                .pointer
                .map(|pointer| TimelineGesture::PointerMoved { x: pointer.x });
        }
        if response.hovered()
            && pointer_over_bands
            && (input.scroll.x != 0.0 || input.scroll.y != 0.0)
        {
            return input.pointer.map(|pointer| TimelineGesture::Scrolled {
                x: pointer.x,
                horizontal_delta: input.scroll.x,
                vertical_delta: input.scroll.y,
                shift_held: input.keys.shift_held,
            });
        }
        None
    }

    fn apply_gesture(
        &mut self,
        snapshot: &TimelineSnapshot,
        transform: TimelineTransform,
        gesture: TimelineGesture,
        output: &mut TimelineRenderOutput,
    ) {
        let Some(interaction) = self.interaction.as_mut() else {
            return;
        };
        let response = interaction.respond(transform, gesture);
        match response.event() {
            Some(TimelineGestureEvent::Clicked { instant }) => {
                let detail = self
                    .pressed_band
                    .and_then(|band| detail_at(snapshot, band, instant));
                self.persistent_detail = detail;
                if detail.is_none() {
                    output.actions.push(TimelineRenderAction::Today(
                        TodayAction::SetTimelineSelection { selection: None },
                    ));
                }
                self.pressed_band = None;
            }
            Some(TimelineGestureEvent::RangeSelected { range }) => {
                output.actions.push(TimelineRenderAction::Today(
                    TodayAction::SetTimelineSelection {
                        selection: Some(range),
                    },
                ));
                self.pressed_band = None;
            }
            Some(TimelineGestureEvent::ScheduleRequested { range }) => {
                output
                    .actions
                    .push(TimelineRenderAction::ScheduleRequested { range });
                self.pressed_band = None;
            }
            Some(TimelineGestureEvent::ViewChanged { range }) => {
                self.auto_follow_default = false;
                output
                    .actions
                    .push(TimelineRenderAction::ViewRangeChanged { range });
                if !contains_range(snapshot.visible_range(), range) {
                    let default_range = initial_view_range(snapshot, self.session_opened_at);
                    self.interaction = Some(TimelineInteraction::new(default_range));
                    self.snapshot_range = Some(snapshot.visible_range());
                    self.default_view_range = Some(default_range);
                    self.auto_follow_default = true;
                }
            }
            Some(TimelineGestureEvent::Cancelled) => {
                self.pressed_band = None;
            }
            Some(
                TimelineGestureEvent::RangePreview { .. }
                | TimelineGestureEvent::SchedulePreview { .. },
            )
            | None => {}
        }
    }

    fn hover_detail(
        &self,
        snapshot: &TimelineSnapshot,
        transform: TimelineTransform,
        band_rects: BandRects,
    ) -> Option<TimelineDetail> {
        let pointer = self.last_pointer?;
        let band = band_rects.band_at(pointer)?;
        detail_at_pointer(snapshot, transform, band, pointer.x)
    }

    fn paint_snapshot(
        &self,
        painter: &Painter,
        snapshot: &TimelineSnapshot,
        transform: TimelineTransform,
        layout: TimelinePaintLayout,
        context: &TodayViewContext,
    ) {
        painter.rect_filled(layout.canvas, 4.0, self.theme_tokens.canvas());
        paint_overview(
            painter,
            layout.overview,
            snapshot,
            transform.visible_range(),
            &self.category_labels,
            layout.overview_hover,
            self.theme_tokens,
        );
        paint_band_labels(painter, layout.bands, self.theme_tokens);
        let plan = TimelinePaintPlan::from_snapshot(transform, snapshot);
        if self.lane_visibility[CATEGORY_LANE] {
            paint_category_band(
                painter,
                plan.category(),
                layout.bands.category,
                &self.category_labels,
                true,
            );
        }
        paint_activity_band(
            painter,
            plan.activity(),
            layout.bands.activity,
            self.lane_visibility[ACTIVE_STATE],
            self.lane_visibility[AWAY_STATE],
            self.lane_visibility[POWERED_OFF_STATE],
        );
        if self.lane_visibility[APPLICATION_LANE] {
            paint_application_band(
                painter,
                plan.application(),
                layout.bands.application,
                &self.application_labels,
            );
        }
        paint_hour_axis(
            painter,
            layout.overview.max.y + OVERVIEW_AXIS_GAP,
            snapshot.visible_range(),
            transform,
            layout.bands,
            self.theme_tokens,
        );
        paint_schedule_overlays(
            painter,
            transform,
            layout.bands,
            snapshot,
            self.theme_tokens,
        );
        paint_selection(
            painter,
            transform,
            layout.bands,
            context.timeline_selection(),
            self.theme_tokens,
        );
        paint_persistent_selection(painter, transform, layout.bands, self.persistent_detail);
    }

    fn paint_persistent_detail(
        &self,
        ui: &mut Ui,
        output: &mut TimelineRenderOutput,
        day_range: HalfOpenInterval,
    ) {
        let Some(detail) = self.persistent_detail else {
            return;
        };
        let detail_label = self.detail_value_label(detail);
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(self.theme_tokens.canvas())
            .stroke(Stroke::new(1.0, self.theme_tokens.timeline_grid()))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.strong("SELECTED DETAIL");
                render_detail_lines(ui, detail, &detail_label, day_range, self.theme_tokens);
                if let (Some(label), Some(action)) =
                    (detail.value().action_label(), detail.action())
                    && ui.button(label).clicked()
                {
                    output.actions.push(TimelineRenderAction::Today(action));
                }
                if let TimelineDetailValue::Application(ApplicationBandValue::Application(
                    application_id,
                )) = detail.value()
                    && ui.button("Edit application category").clicked()
                {
                    output
                        .actions
                        .push(TimelineRenderAction::OpenCategories { application_id });
                }
            });
    }

    fn detail_value_label(&self, detail: TimelineDetail) -> String {
        match detail.value() {
            TimelineDetailValue::Category(CategoryBandValue::Category(category_id)) => self
                .category_labels
                .get(&category_id)
                .cloned()
                .unwrap_or_else(|| detail.value().label()),
            TimelineDetailValue::Application(ApplicationBandValue::Application(application_id)) => {
                self.application_labels
                    .get(&application_id)
                    .cloned()
                    .unwrap_or_else(|| detail.value().label())
            }
            _ => detail.value().label(),
        }
    }
}

fn paint_schedule_overlays(
    painter: &Painter,
    transform: TimelineTransform,
    band_rects: BandRects,
    snapshot: &TimelineSnapshot,
    theme_tokens: ThemeTokens,
) {
    for overlay in prepare_schedule_overlays(
        transform,
        snapshot
            .schedule_occurrences()
            .iter()
            .map(|occurrence| (occurrence, occurrence.interval())),
    ) {
        let adjusted = overlay.occurrence().adjusted();
        let pixels = overlay.bracket().range().pixels();
        let top = band_rects.category.min.y + 2.0;
        let bottom = band_rects.application.max.y - 2.0;
        let stroke = Stroke::new(
            if adjusted { 2.0 } else { 1.0 },
            theme_tokens.schedule_bracket(),
        );
        painter.line_segment(
            [
                Pos2::new(pixels.start_x(), top),
                Pos2::new(pixels.end_x(), top),
            ],
            stroke,
        );
        painter.line_segment(
            [
                Pos2::new(pixels.start_x(), top),
                Pos2::new(pixels.start_x(), bottom),
            ],
            stroke,
        );
        painter.line_segment(
            [
                Pos2::new(pixels.end_x(), top),
                Pos2::new(pixels.end_x(), bottom),
            ],
            stroke,
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct BandRects {
    category: Rect,
    activity: Rect,
    application: Rect,
}

#[derive(Clone, Copy, Debug)]
struct TimelinePaintLayout {
    canvas: Rect,
    overview: Rect,
    overview_hover: Option<OverviewHover>,
    bands: BandRects,
}

#[derive(Clone, Copy, Debug)]
enum OverviewCapture {
    Pan {
        initial: HalfOpenInterval,
        grab_offset_us: i128,
    },
    ResizeStart {
        initial: HalfOpenInterval,
    },
    ResizeEnd {
        initial: HalfOpenInterval,
    },
}

impl OverviewCapture {
    const fn cursor_icon(self) -> CursorIcon {
        match self {
            Self::Pan { .. } => CursorIcon::Grabbing,
            Self::ResizeStart { .. } | Self::ResizeEnd { .. } => CursorIcon::ResizeHorizontal,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverviewHover {
    Body,
    StartEdge,
    EndEdge,
}

impl OverviewHover {
    const fn cursor_icon(self) -> CursorIcon {
        match self {
            Self::Body => CursorIcon::Grab,
            Self::StartEdge | Self::EndEdge => CursorIcon::ResizeHorizontal,
        }
    }
}

fn overview_hover_at_pointer(
    pointer: Pos2,
    overview_rect: Rect,
    day_range: HalfOpenInterval,
    view_range: HalfOpenInterval,
) -> Option<OverviewHover> {
    if !overview_rect.contains(pointer) {
        return None;
    }
    let viewport = overview_view_rect(overview_rect, day_range, view_range)?;
    if (pointer.x - viewport.left()).abs() <= OVERVIEW_EDGE_GRAB_WIDTH {
        Some(OverviewHover::StartEdge)
    } else if (pointer.x - viewport.right()).abs() <= OVERVIEW_EDGE_GRAB_WIDTH {
        Some(OverviewHover::EndEdge)
    } else if viewport.contains(pointer) {
        Some(OverviewHover::Body)
    } else {
        None
    }
}

fn overview_capture_at_pointer(
    pointer: Pos2,
    pointer_instant: openmanic_domain::UtcMicros,
    viewport: Rect,
    view_range: HalfOpenInterval,
) -> OverviewCapture {
    if (pointer.x - viewport.left()).abs() <= OVERVIEW_EDGE_GRAB_WIDTH {
        return OverviewCapture::ResizeStart {
            initial: view_range,
        };
    }
    if (pointer.x - viewport.right()).abs() <= OVERVIEW_EDGE_GRAB_WIDTH {
        return OverviewCapture::ResizeEnd {
            initial: view_range,
        };
    }
    let grab_offset_us = if viewport.contains(pointer) {
        i128::from(pointer_instant.get()) - i128::from(view_range.start().get())
    } else {
        i128::from(view_range.duration_us() / 2)
    };
    OverviewCapture::Pan {
        initial: view_range,
        grab_offset_us,
    }
}

impl BandRects {
    fn new(timeline_rect: Rect) -> Self {
        let category = Rect::from_min_size(
            timeline_rect.min,
            Vec2::new(timeline_rect.width(), CATEGORY_BAND_HEIGHT),
        );
        let activity = Rect::from_min_size(
            Pos2::new(category.min.x, category.max.y + BAND_GAP),
            Vec2::new(timeline_rect.width(), ACTIVITY_BAND_HEIGHT),
        );
        let application = Rect::from_min_size(
            Pos2::new(activity.min.x, activity.max.y + BAND_GAP),
            Vec2::new(timeline_rect.width(), APPLICATION_BAND_HEIGHT),
        );
        Self {
            category,
            activity,
            application,
        }
    }

    fn contains(self, pointer: Pos2) -> bool {
        self.category.contains(pointer)
            || self.activity.contains(pointer)
            || self.application.contains(pointer)
    }

    fn band_at(self, pointer: Pos2) -> Option<TimelineBand> {
        if self.category.contains(pointer) {
            Some(TimelineBand::Category)
        } else if self.activity.contains(pointer) {
            Some(TimelineBand::Activity)
        } else if self.application.contains(pointer) {
            Some(TimelineBand::Application)
        } else {
            None
        }
    }

    const fn rect_for(self, band: TimelineBand) -> Rect {
        match band {
            TimelineBand::Category => self.category,
            TimelineBand::Activity => self.activity,
            TimelineBand::Application => self.application,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TimelineInput {
    pointer: Option<Pos2>,
    buttons: PointerButtons,
    scroll: Vec2,
    keys: TimelineKeys,
}

#[derive(Clone, Copy, Debug)]
struct PointerButtons {
    primary_down: bool,
    middle_down: bool,
}

#[derive(Clone, Copy, Debug)]
struct TimelineKeys {
    shift_held: bool,
    space_held: bool,
    escape_pressed: bool,
}

impl TimelineInput {
    fn from_input(input: &egui::InputState) -> Self {
        Self {
            pointer: input.pointer.latest_pos(),
            buttons: PointerButtons {
                primary_down: input.pointer.button_down(PointerButton::Primary),
                middle_down: input.pointer.button_down(PointerButton::Middle),
            },
            scroll: input.smooth_scroll_delta,
            keys: TimelineKeys {
                shift_held: input.modifiers.shift,
                space_held: input.key_down(Key::Space),
                escape_pressed: input.key_pressed(Key::Escape),
            },
        }
    }
}

fn detail_at(
    snapshot: &TimelineSnapshot,
    band: TimelineBand,
    instant: openmanic_domain::UtcMicros,
) -> Option<TimelineDetail> {
    match band {
        TimelineBand::Category => {
            let interval = snapshot.category_band().at(instant)?;
            Some(TimelineDetail::new(
                band,
                TimelineDetailValue::Category(*interval.value()),
                instant,
                interval.range(),
                interval.raw_fragments(),
            ))
        }
        TimelineBand::Activity => {
            let interval = snapshot.activity_band().at(instant)?;
            Some(TimelineDetail::new(
                band,
                TimelineDetailValue::Activity(*interval.value()),
                instant,
                interval.range(),
                interval.raw_fragments(),
            ))
        }
        TimelineBand::Application => {
            let interval = snapshot.application_band().at(instant)?;
            Some(TimelineDetail::new(
                band,
                TimelineDetailValue::Application(*interval.value()),
                instant,
                interval.range(),
                interval.raw_fragments(),
            ))
        }
    }
}

fn detail_at_pointer(
    snapshot: &TimelineSnapshot,
    transform: TimelineTransform,
    band: TimelineBand,
    pointer_x: f32,
) -> Option<TimelineDetail> {
    match band {
        TimelineBand::Category => {
            let hit = hit_test(transform, snapshot.category_band(), pointer_x)?;
            detail_at(snapshot, band, hit.instant())
        }
        TimelineBand::Activity => {
            let hit = hit_test(transform, snapshot.activity_band(), pointer_x)?;
            detail_at(snapshot, band, hit.instant())
        }
        TimelineBand::Application => {
            let hit = hit_test(transform, snapshot.application_band(), pointer_x)?;
            detail_at(snapshot, band, hit.instant())
        }
    }
}

fn contains_range(outer: HalfOpenInterval, inner: HalfOpenInterval) -> bool {
    outer.start() <= inner.start() && inner.end() <= outer.end()
}

fn overview_view_rect(
    overview_rect: Rect,
    day_range: HalfOpenInterval,
    view_range: HalfOpenInterval,
) -> Option<Rect> {
    if !contains_range(day_range, view_range) {
        return None;
    }
    let transform =
        TimelineTransform::try_new(day_range, overview_rect.min.x, overview_rect.width()).ok()?;
    Some(Rect::from_min_max(
        Pos2::new(transform.x_for(view_range.start()), overview_rect.min.y),
        Pos2::new(transform.x_for(view_range.end()), overview_rect.max.y),
    ))
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "overview pointer geometry is converted to a bounded microsecond offset"
)]
fn overview_instant_at_x(
    overview_rect: Rect,
    day_range: HalfOpenInterval,
    x: f32,
) -> Option<openmanic_domain::UtcMicros> {
    if !x.is_finite() || overview_rect.width() <= 0.0 {
        return None;
    }
    let ratio = f64::from((x - overview_rect.left()) / overview_rect.width()).clamp(0.0, 1.0);
    let offset = (day_range.duration_us() as f64 * ratio).round() as i128;
    let value = i128::from(day_range.start().get()).saturating_add(offset);
    Some(openmanic_domain::UtcMicros::new(
        i64::try_from(value)
            .ok()?
            .clamp(day_range.start().get(), day_range.end().get()),
    ))
}

fn overview_range_for_pointer(
    overview_rect: Rect,
    day_range: HalfOpenInterval,
    capture: OverviewCapture,
    pointer_x: f32,
) -> Option<HalfOpenInterval> {
    let pointer = i128::from(overview_instant_at_x(overview_rect, day_range, pointer_x)?.get());
    let day_start = i128::from(day_range.start().get());
    let day_end = i128::from(day_range.end().get());
    let minimum_duration = i128::from(MIN_NAVIGATOR_RANGE_US.min(day_range.duration_us()));
    let (start, end) = match capture {
        OverviewCapture::Pan {
            initial,
            grab_offset_us,
        } => {
            let duration = i128::from(initial.duration_us());
            let start = pointer
                .saturating_sub(grab_offset_us)
                .clamp(day_start, day_end.saturating_sub(duration));
            (start, start.saturating_add(duration))
        }
        OverviewCapture::ResizeStart { initial } => {
            let end = i128::from(initial.end().get());
            let required = minimum_duration.min(end.saturating_sub(day_start));
            (pointer.clamp(day_start, end.saturating_sub(required)), end)
        }
        OverviewCapture::ResizeEnd { initial } => {
            let start = i128::from(initial.start().get());
            let required = minimum_duration.min(day_end.saturating_sub(start));
            (
                start,
                pointer.clamp(start.saturating_add(required), day_end),
            )
        }
    };
    HalfOpenInterval::try_new(
        openmanic_domain::UtcMicros::new(i64::try_from(start).ok()?),
        openmanic_domain::UtcMicros::new(i64::try_from(end).ok()?),
    )
    .ok()
}

fn paint_band_labels(painter: &Painter, bands: BandRects, theme_tokens: ThemeTokens) {
    for (band, rect) in [
        (TimelineBand::Category, bands.category),
        (TimelineBand::Activity, bands.activity),
        (TimelineBand::Application, bands.application),
    ] {
        painter.text(
            Pos2::new(rect.min.x - BAND_LABEL_WIDTH + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            band.label(),
            FontId::proportional(if band == TimelineBand::Category {
                12.0
            } else {
                10.0
            }),
            theme_tokens.content_primary(),
        );
        painter.rect_filled(rect, 4.0, TRACK);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, theme_tokens.timeline_grid()),
            egui::StrokeKind::Inside,
        );
    }
}

fn paint_overview(
    painter: &Painter,
    overview_rect: Rect,
    snapshot: &TimelineSnapshot,
    view_range: HalfOpenInterval,
    category_labels: &BTreeMap<CategoryId, String>,
    hover: Option<OverviewHover>,
    theme_tokens: ThemeTokens,
) {
    painter.text(
        Pos2::new(
            overview_rect.min.x - BAND_LABEL_WIDTH + 8.0,
            overview_rect.min.y - 9.0,
        ),
        Align2::LEFT_CENTER,
        "24-HOUR VIEWFINDER",
        FontId::proportional(10.0),
        theme_tokens.content_secondary(),
    );
    painter.text(
        Pos2::new(overview_rect.max.x, overview_rect.min.y - 9.0),
        Align2::RIGHT_CENTER,
        format!(
            "View: {} UTC",
            format_detail_range(view_range, snapshot.visible_range())
        ),
        FontId::proportional(10.0),
        theme_tokens.interaction_primary(),
    );
    painter.rect_filled(overview_rect, 4.0, SUBWIDGET);
    painter.rect_stroke(
        overview_rect,
        4.0,
        Stroke::new(1.0, BAND_BORDER),
        egui::StrokeKind::Inside,
    );
    let Ok(day_transform) = TimelineTransform::try_new(
        snapshot.visible_range(),
        overview_rect.min.x,
        overview_rect.width(),
    ) else {
        return;
    };
    let plan = TimelinePaintPlan::from_snapshot(day_transform, snapshot);
    paint_overview_day_map(painter, overview_rect, &plan, category_labels);
    let Some(viewport) = overview_view_rect(overview_rect, snapshot.visible_range(), view_range)
    else {
        return;
    };
    paint_overview_window(painter, viewport, hover);
}

fn paint_overview_day_map(
    painter: &Painter,
    overview_rect: Rect,
    plan: &TimelinePaintPlan<'_>,
    category_labels: &BTreeMap<CategoryId, String>,
) {
    let day_map_rect = Rect::from_min_max(
        Pos2::new(overview_rect.left(), overview_rect.top() + 2.0),
        Pos2::new(overview_rect.right(), overview_rect.bottom() - 2.0),
    );
    paint_band(
        painter,
        plan.category(),
        day_map_rect,
        0.0,
        0.0,
        |value| {
            let color = match value {
                CategoryBandValue::Category(category_id) => {
                    category_color_for(*category_id, category_labels)
                }
                CategoryBandValue::Uncategorized => Color32::from_rgb(51, 65, 85),
                CategoryBandValue::NonApplicationState(state) => activity_color(*state),
            };
            color.gamma_multiply(0.30)
        },
        |_| None,
    );
}

fn paint_overview_window(painter: &Painter, viewport: Rect, hover: Option<OverviewHover>) {
    painter.rect_filled(
        viewport,
        0.0,
        VIEWFINDER_ACCENT.gamma_multiply(if hover == Some(OverviewHover::Body) {
            0.20
        } else {
            0.15
        }),
    );
    for (x, edge) in [
        (viewport.left(), OverviewHover::StartEdge),
        (viewport.right(), OverviewHover::EndEdge),
    ] {
        painter.line_segment(
            [
                Pos2::new(x, viewport.top()),
                Pos2::new(x, viewport.bottom()),
            ],
            Stroke::new(
                if hover == Some(edge) { 3.0 } else { 2.0 },
                VIEWFINDER_ACCENT,
            ),
        );
    }
}

fn initial_view_range(
    snapshot: &TimelineSnapshot,
    session_opened_at: UtcMicros,
) -> HalfOpenInterval {
    let day_range = snapshot.visible_range();
    let tracked_bounds = snapshot
        .application_band()
        .intervals()
        .iter()
        .filter(|interval| matches!(interval.value(), ApplicationBandValue::Application(_)))
        .flat_map(openmanic_application::TimelineInterval::raw_fragments)
        .fold(None::<(UtcMicros, UtcMicros)>, |bounds, fragment| {
            let range = fragment.visible_range();
            Some(
                bounds.map_or((range.start(), range.end()), |(first, last)| {
                    (first.min(range.start()), last.max(range.end()))
                }),
            )
        });

    let latest_possible_start = UtcMicros::new(day_range.end().get().saturating_sub(1));
    let anchor = tracked_bounds.map_or(session_opened_at, |(first, _)| first);
    let start = anchor.clamp(day_range.start(), latest_possible_start);
    let minimum_end = add_hours(start, INITIAL_VIEW_HOURS).min(day_range.end());
    let tracked_end = tracked_bounds.map_or(minimum_end, |(_, last)| {
        add_hours(last, INITIAL_VIEW_TRAILING_HOURS)
    });
    let end = minimum_end.max(tracked_end).min(day_range.end());

    HalfOpenInterval::try_new(start, end).unwrap_or(day_range)
}

fn add_hours(instant: UtcMicros, hours: u64) -> UtcMicros {
    let micros = hours.saturating_mul(HOUR_US as u64);
    let offset = i64::try_from(micros).unwrap_or(i64::MAX);
    UtcMicros::new(instant.get().saturating_add(offset))
}

fn paint_hour_axis(
    painter: &Painter,
    axis_top: f32,
    day_range: HalfOpenInterval,
    transform: TimelineTransform,
    bands: BandRects,
    theme_tokens: ThemeTokens,
) {
    const TICK_INTERVALS: u64 = 5;
    let visible = transform.visible_range();
    for tick_index in 0..=TICK_INTERVALS {
        let offset = visible.duration_us().saturating_mul(tick_index) / TICK_INTERVALS;
        let instant = UtcMicros::new(
            visible
                .start()
                .get()
                .saturating_add(i64::try_from(offset).unwrap_or(i64::MAX)),
        );
        let x = transform.x_for(instant);
        let alignment = if tick_index == 0 {
            Align2::LEFT_TOP
        } else if tick_index == TICK_INTERVALS {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            Pos2::new(x, axis_top + 2.0),
            alignment,
            format_day_clock(day_range, instant),
            FontId::proportional(10.0),
            theme_tokens.content_secondary(),
        );
        painter.line_segment(
            [
                Pos2::new(x, bands.category.min.y),
                Pos2::new(x, bands.application.max.y),
            ],
            Stroke::new(0.75, theme_tokens.timeline_grid()),
        );
    }
}

#[cfg(test)]
fn hour_label(hour_index: u64) -> String {
    let hour = hour_index % 24;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let display = match hour % 12 {
        0 => 12,
        value => value,
    };
    format!("{display} {suffix}")
}

fn paint_category_band(
    painter: &Painter,
    band: &super::PaintBand<'_, CategoryBandValue>,
    rect: Rect,
    labels: &BTreeMap<CategoryId, String>,
    show_labels: bool,
) {
    paint_band(
        painter,
        band,
        rect,
        6.0,
        1.5,
        |value| match value {
            CategoryBandValue::Category(category_id) => category_color_for(*category_id, labels)
                .gamma_multiply(if show_labels { 1.0 } else { 0.3 }),
            CategoryBandValue::Uncategorized => {
                Color32::from_rgb(51, 65, 85).gamma_multiply(if show_labels { 1.0 } else { 0.3 })
            }
            CategoryBandValue::NonApplicationState(state) => activity_color(*state),
        },
        |value| {
            show_labels.then(|| match value {
                CategoryBandValue::Category(category_id) => labels
                    .get(category_id)
                    .cloned()
                    .unwrap_or_else(|| "Category".to_owned()),
                CategoryBandValue::Uncategorized => "Uncategorized".to_owned(),
                CategoryBandValue::NonApplicationState(state) => activity_label(*state).to_owned(),
            })
        },
    );
}

fn paint_activity_band(
    painter: &Painter,
    band: &super::PaintBand<'_, ActivityStateValue>,
    rect: Rect,
    show_active: bool,
    show_away: bool,
    show_powered_off: bool,
) {
    paint_band(
        painter,
        band,
        rect,
        3.0,
        1.0,
        |value| match value.state() {
            ActivityState::Active if !show_active => Color32::TRANSPARENT,
            ActivityState::Idle if !show_away => Color32::TRANSPARENT,
            ActivityState::PoweredOff if !show_powered_off => Color32::TRANSPARENT,
            state => activity_color(state),
        },
        |_| None,
    );
}

fn paint_application_band(
    painter: &Painter,
    band: &super::PaintBand<'_, ApplicationBandValue>,
    rect: Rect,
    labels: &BTreeMap<ApplicationId, String>,
) {
    paint_band(
        painter,
        band,
        rect,
        3.0,
        1.0,
        |value| match value {
            ApplicationBandValue::Application(application_id) => {
                application_color_for(*application_id, labels)
            }
            ApplicationBandValue::NoApplication(state) => activity_color(*state),
            ApplicationBandValue::UnresolvedApplication => Color32::from_rgb(150, 111, 184),
        },
        |value| match value {
            ApplicationBandValue::Application(application_id) => {
                labels.get(application_id).cloned()
            }
            ApplicationBandValue::NoApplication(_) => None,
            ApplicationBandValue::UnresolvedApplication => Some("Unknown app".to_owned()),
        },
    );
}

fn paint_band<T>(
    painter: &Painter,
    band: &super::PaintBand<'_, T>,
    rect: Rect,
    corner_radius: f32,
    horizontal_inset: f32,
    color_for: impl Fn(&T) -> Color32,
    label_for: impl Fn(&T) -> Option<String>,
) {
    for primitive in band.primitives() {
        match primitive {
            PaintPrimitive::Segment(segment) => {
                let segment_rect = paint_primitive(
                    painter,
                    rect,
                    segment.geometry().pixels(),
                    segment.fill(),
                    color_for(segment.interval().value()),
                    corner_radius,
                    horizontal_inset,
                );
                if segment.fill() == PaintFill::Visible {
                    paint_segment_label(
                        painter,
                        segment_rect,
                        label_for(segment.interval().value()),
                    );
                }
            }
            PaintPrimitive::Dense(bin) => {
                let Some(color) = bin
                    .sources()
                    .iter()
                    .find(|source| source.fill() == PaintFill::Visible)
                    .map(|source| color_for(source.interval().value()))
                else {
                    continue;
                };
                paint_primitive(
                    painter,
                    rect,
                    bin.pixels(),
                    PaintFill::Visible,
                    color,
                    corner_radius,
                    horizontal_inset,
                );
            }
        }
    }
}

fn paint_primitive(
    painter: &Painter,
    band_rect: Rect,
    pixels: super::PixelRange,
    fill: PaintFill,
    color: Color32,
    corner_radius: f32,
    horizontal_inset: f32,
) -> Rect {
    let mut rect = Rect::from_min_max(
        Pos2::new(pixels.start_x(), band_rect.min.y + 1.0),
        Pos2::new(
            pixels.end_x().max(pixels.start_x() + 1.0),
            band_rect.max.y - 1.0,
        ),
    );
    if rect.width() > horizontal_inset * 2.0 + 1.0 {
        rect.min.x += horizontal_inset;
        rect.max.x -= horizontal_inset;
    }
    if fill == PaintFill::Visible {
        let rounding = corner_radius.min(rect.height() / 2.0);
        if rect.height() >= 10.0 && rect.width() >= 3.0 {
            // Design cell-gradient rule: darker top, lighter bottom.
            painter.rect_filled(rect, rounding, crate::design::shade(color, -0.26));
            crate::design::paint_cell_gradient(painter, rect.shrink(0.5), color);
        } else {
            painter.rect_filled(rect, rounding, color);
        }
    } else {
        painter.line_segment(
            [rect.left_top(), rect.right_bottom()],
            Stroke::new(1.0, BAND_BORDER),
        );
        painter.line_segment(
            [rect.left_bottom(), rect.right_top()],
            Stroke::new(1.0, BAND_BORDER),
        );
    }
    rect
}

fn fitted_segment_label(label: &str, width: f32) -> String {
    let maximum = if width < 72.0 {
        8
    } else if width < 110.0 {
        14
    } else {
        24
    };
    if label.chars().count() <= maximum {
        return label.to_owned();
    }
    let kept = maximum.saturating_sub(1);
    format!("{}…", label.chars().take(kept).collect::<String>())
}

fn paint_segment_label(painter: &Painter, rect: Rect, label: Option<String>) {
    if rect.width() < 48.0 {
        return;
    }
    let Some(label) = label else {
        return;
    };
    let font_size = if rect.height() >= 24.0 { 11.0 } else { 8.0 };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        fitted_segment_label(&label, rect.width()),
        FontId::proportional(font_size),
        Color32::WHITE,
    );
}

const fn activity_label(state: ActivityState) -> &'static str {
    match state {
        ActivityState::Active => "Active",
        ActivityState::Idle => "Away",
        ActivityState::PausedByUser => "Paused",
        ActivityState::Excluded => "Excluded",
        ActivityState::Unavailable => "Unavailable",
        ActivityState::PoweredOff => "Powered off",
        ActivityState::UnknownMissing => "Unknown",
    }
}

fn paint_selection(
    painter: &Painter,
    transform: TimelineTransform,
    bands: BandRects,
    selection: Option<HalfOpenInterval>,
    theme_tokens: ThemeTokens,
) {
    let Some(selection) = selection.and_then(|range| transform.range_geometry(range)) else {
        return;
    };
    let highlight = Rect::from_min_max(
        Pos2::new(selection.pixels().start_x(), bands.category.min.y),
        Pos2::new(selection.pixels().end_x(), bands.application.max.y),
    );
    painter.rect_filled(
        highlight,
        3.0,
        theme_tokens.interaction_primary().gamma_multiply(0.25),
    );
    paint_dashed_outline(
        painter,
        highlight,
        Stroke::new(1.0, theme_tokens.interaction_primary()),
    );
}

fn paint_persistent_selection(
    painter: &Painter,
    transform: TimelineTransform,
    bands: BandRects,
    detail: Option<TimelineDetail>,
) {
    let Some(detail) = detail else {
        return;
    };
    let Some(geometry) = transform.range_geometry(detail.interval_range()) else {
        return;
    };
    let band = bands.rect_for(detail.band());
    let rect = Rect::from_min_max(
        Pos2::new(geometry.pixels().start_x(), band.min.y + 1.0),
        Pos2::new(geometry.pixels().end_x(), band.max.y - 1.0),
    );
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.5, SELECTED),
        egui::StrokeKind::Inside,
    );
}

fn paint_dashed_outline(painter: &Painter, rect: Rect, stroke: Stroke) {
    const DASH: f32 = 5.0;
    const GAP: f32 = 3.0;
    for (start, end) in [
        (rect.left_top(), rect.right_top()),
        (rect.left_bottom(), rect.right_bottom()),
        (rect.left_top(), rect.left_bottom()),
        (rect.right_top(), rect.right_bottom()),
    ] {
        let delta = end - start;
        let length = delta.length();
        if length <= 0.0 {
            continue;
        }
        let direction = delta / length;
        let mut offset = 0.0;
        while offset < length {
            let dash_end = (offset + DASH).min(length);
            painter.line_segment(
                [start + direction * offset, start + direction * dash_end],
                stroke,
            );
            offset += DASH + GAP;
        }
    }
}

fn render_timeline_legend(
    ui: &mut Ui,
    snapshot: &TimelineSnapshot,
    theme_tokens: ThemeTokens,
    visibility: &mut [bool; 5],
) {
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(theme_tokens.content_secondary(), "LEGEND");
        ui.checkbox(&mut visibility[CATEGORY_LANE], "Category");
        ui.checkbox(&mut visibility[ACTIVE_STATE], "Active");
        ui.checkbox(&mut visibility[AWAY_STATE], "Away");
        ui.checkbox(&mut visibility[APPLICATION_LANE], "Applications");
        ui.checkbox(&mut visibility[POWERED_OFF_STATE], "Powered off");
        if let DataCompleteness::Partial {
            unknown_missing_duration_us,
        } = snapshot.completeness()
        {
            let covered = snapshot
                .visible_range()
                .duration_us()
                .saturating_sub(unknown_missing_duration_us);
            ui.colored_label(
                theme_tokens.content_secondary(),
                format!("Recorded: {}", format_duration(covered)),
            );
        }
    });
}

#[expect(dead_code, reason = "superseded by the compact timeline legend")]
fn paint_completeness(painter: &Painter, canvas: Rect, completeness: DataCompleteness) {
    let DataCompleteness::Partial {
        unknown_missing_duration_us,
    } = completeness
    else {
        return;
    };
    painter.text(
        Pos2::new(canvas.max.x - 8.0, canvas.min.y + 6.0),
        Align2::RIGHT_TOP,
        format!(
            "Missing activity: {}",
            format_duration(unknown_missing_duration_us)
        ),
        FontId::proportional(10.0),
        WARNING,
    );
}

#[expect(
    dead_code,
    reason = "retained temporarily for legacy diagnostic compatibility"
)]
fn paint_completeness_legacy(painter: &Painter, canvas: Rect, completeness: DataCompleteness) {
    let DataCompleteness::Partial {
        unknown_missing_duration_us,
    } = completeness
    else {
        return;
    };
    painter.text(
        Pos2::new(canvas.max.x - 6.0, canvas.min.y + 4.0),
        Align2::RIGHT_TOP,
        format!("Unknown activity: {unknown_missing_duration_us} µs"),
        FontId::proportional(10.0),
        WARNING,
    );
}

fn paint_hover_detail(
    canvas_painter: &Painter,
    clip: Rect,
    pointer: Pos2,
    detail: TimelineDetail,
    detail_label: &str,
    day_range: HalfOpenInterval,
    theme_tokens: ThemeTokens,
) {
    let text = detail_text(detail, detail_label, day_range);
    let height = (3.0 * DETAIL_LINE_HEIGHT) + 20.0;
    let x = if pointer.x + 16.0 + DETAIL_WIDTH <= clip.max.x {
        pointer.x + 16.0
    } else {
        (pointer.x - DETAIL_WIDTH - 16.0).max(clip.min.x)
    };
    let y = (pointer.y + 16.0).min(clip.max.y - height).max(clip.min.y);
    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(DETAIL_WIDTH, height));
    canvas_painter.rect_filled(rect, 8.0, theme_tokens.panel());
    canvas_painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme_tokens.interaction_primary()),
        egui::StrokeKind::Inside,
    );
    canvas_painter.text(
        rect.min + Vec2::new(10.0, 9.0),
        Align2::LEFT_TOP,
        text,
        FontId::proportional(12.0),
        theme_tokens.content_primary(),
    );
}

fn render_detail_lines(
    ui: &mut Ui,
    detail: TimelineDetail,
    detail_label: &str,
    day_range: HalfOpenInterval,
    theme_tokens: ThemeTokens,
) {
    ui.colored_label(theme_tokens.content_secondary(), detail.band().label());
    ui.strong(detail_label);
    ui.label(format_detail_range(detail.interval_range(), day_range));
    ui.colored_label(
        theme_tokens.content_secondary(),
        format_duration(detail.interval_range().duration_us()),
    );
}

fn detail_text(detail: TimelineDetail, detail_label: &str, day_range: HalfOpenInterval) -> String {
    format!(
        "{}  /  {}\n{}\n{}",
        detail.band().label(),
        detail_label,
        format_detail_range(detail.interval_range(), day_range),
        format_duration(detail.interval_range().duration_us()),
    )
}

fn format_detail_range(range: HalfOpenInterval, day_range: HalfOpenInterval) -> String {
    format!(
        "{} - {}",
        format_day_clock(day_range, range.start()),
        format_day_clock(day_range, range.end()),
    )
}

fn format_day_clock(day_range: HalfOpenInterval, instant: openmanic_domain::UtcMicros) -> String {
    let elapsed_us = i128::from(instant.get())
        .saturating_sub(i128::from(day_range.start().get()))
        .max(0);
    let total_seconds = u64::try_from(elapsed_us / 1_000_000).unwrap_or(u64::MAX);
    let hour = (total_seconds / 3_600) % 24;
    let minute = (total_seconds % 3_600) / 60;
    let second = total_seconds % 60;
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    if second == 0 {
        format!("{display_hour}:{minute:02} {suffix}")
    } else {
        format!("{display_hour}:{minute:02}:{second:02} {suffix}")
    }
}

fn format_duration(duration_us: u64) -> String {
    let seconds = duration_us / 1_000_000;
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn render_presentation_notice(ui: &mut Ui, data: &PresentableData<TimelineSnapshot>) {
    let (message, color) = match data {
        PresentableData::InitialLoading | PresentableData::Ready(_) | PresentableData::Empty(_) => {
            return;
        }
        PresentableData::Refreshing { .. } => (
            "Refreshing timeline. Current data remains visible.",
            WARNING,
        ),
        PresentableData::Partial { limitations, .. } => (partial_message(limitations), WARNING),
        PresentableData::Failed { .. } => (
            "Timeline data could not be refreshed. Showing the last available data when possible.",
            ERROR,
        ),
        PresentableData::Recovered { notice, .. } => (notice.as_str(), SUCCESS),
    };
    ui.colored_label(color, message);
}

fn render_unavailable_canvas(ui: &mut Ui, message: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width().max(BAND_LABEL_WIDTH + 1.0),
            TIMELINE_HEIGHT,
        ),
        Sense::hover(),
    );
    ui.painter().rect_filled(rect, 4.0, SUBWIDGET);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        message,
        FontId::proportional(14.0),
        MUTED,
    );
}

fn unavailable_message(data: &PresentableData<TimelineSnapshot>) -> &'static str {
    match data {
        PresentableData::InitialLoading => "Loading timeline…",
        PresentableData::Empty(reason) => reason.message(),
        PresentableData::Failed { .. } => "Timeline data is unavailable. Try again.",
        PresentableData::Ready(_)
        | PresentableData::Refreshing { .. }
        | PresentableData::Partial { .. }
        | PresentableData::Recovered { .. } => "Timeline data is unavailable.",
    }
}

fn partial_message(limitations: &[DataLimitation]) -> &'static str {
    match limitations.first() {
        Some(DataLimitation::TrackingPaused) => "Timeline is partial: tracking was paused.",
        Some(DataLimitation::TrackingUnavailable) => {
            "Timeline is partial: tracking was unavailable."
        }
        Some(DataLimitation::StillLoading) => "Timeline is partial: more values are loading.",
        None => "Timeline is partial.",
    }
}

const fn activity_color(state: ActivityState) -> Color32 {
    match state {
        ActivityState::Active => crate::design::ACTIVE,
        ActivityState::Idle => crate::design::AWAY,
        ActivityState::PausedByUser => Color32::from_rgb(92, 119, 255),
        ActivityState::Excluded => Color32::from_rgb(174, 92, 255),
        ActivityState::Unavailable | ActivityState::PoweredOff => crate::design::POWERED_OFF,
        ActivityState::UnknownMissing => crate::design::UNKNOWN,
    }
}

fn category_color_for(category_id: CategoryId, labels: &BTreeMap<CategoryId, String>) -> Color32 {
    let Some(label) = labels.get(&category_id) else {
        return category_color(category_id.as_bytes());
    };
    if label.eq_ignore_ascii_case("uncategorized") {
        return crate::design::UNKNOWN;
    }
    let resolved = crate::design::category_color(label);
    if resolved == crate::design::UNKNOWN {
        category_color(category_id.as_bytes())
    } else {
        resolved
    }
}

fn application_color_for(
    application_id: ApplicationId,
    labels: &BTreeMap<ApplicationId, String>,
) -> Color32 {
    let Some(label) = labels.get(&application_id) else {
        return application_color(application_id.as_bytes());
    };
    if let Some(brand) = crate::design::application_brand_color(label) {
        return brand;
    }
    let normalized = label.to_ascii_lowercase();
    if normalized.contains("firefox") {
        Color32::from_rgb(249, 115, 22)
    } else if normalized.contains("spotify") {
        Color32::from_rgb(34, 197, 94)
    } else if normalized.contains("chatgpt") || normalized.contains("gemini") {
        Color32::from_rgb(168, 85, 247)
    } else if normalized.contains("mpv") || normalized.contains("vlc") {
        Color32::from_rgb(236, 72, 153)
    } else if normalized.contains("keepass") || normalized.contains("1password") {
        Color32::from_rgb(20, 184, 166)
    } else {
        application_color(application_id.as_bytes())
    }
}

const fn category_color(bytes: [u8; 16]) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(103, 84, 255),
        Color32::from_rgb(21, 184, 214),
        Color32::from_rgb(225, 66, 156),
        Color32::from_rgb(239, 119, 30),
        Color32::from_rgb(81, 104, 220),
        Color32::from_rgb(151, 79, 214),
    ];
    PALETTE[(bytes[0] as usize + bytes[5] as usize) % PALETTE.len()]
}

const fn application_color(bytes: [u8; 16]) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(103, 84, 255),
        Color32::from_rgb(21, 184, 214),
        Color32::from_rgb(225, 66, 156),
        Color32::from_rgb(239, 119, 30),
        Color32::from_rgb(81, 104, 220),
        Color32::from_rgb(21, 201, 152),
    ];
    PALETTE[(bytes[2] as usize + bytes[9] as usize) % PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Rect, pos2};
    use openmanic_application::{
        DataRevision, ProjectionContextKey, TimelineApplication, TimelineContext,
        TimelineProjectionSource, TimelineProjector, TimelineRawIntervalId, TimelineSourceActivity,
    };
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, ApplicationId,
        HalfOpenInterval, PowerTransitionEvidence, TrackerRunId, UtcMicros,
    };

    use super::{
        HOUR_US, OverviewCapture, OverviewHover, TimelineBand, TimelineDetailValue,
        TimelineRenderAction, detail_at, format_day_clock, hour_label, initial_view_range,
        overview_hover_at_pointer, overview_range_for_pointer, overview_view_rect,
    };
    use crate::TodayAction;

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("fixture range is positive")
    }

    fn source(raw_id: u64, start: i64, end: i64, state: ActivityState) -> TimelineSourceActivity {
        let evidence = match state {
            ActivityState::PoweredOff => ActivityEvidence::confirmed_shutdown(
                PowerTransitionEvidence::try_new(UtcMicros::new(start), UtcMicros::new(end))
                    .expect("powered-off fixture has a positive evidence span"),
            ),
            _ => ActivityEvidence::try_from_cause(ActivityCause::IdleThreshold)
                .expect("ordinary fixture state accepts ordinary evidence"),
        };
        TimelineSourceActivity::new(
            TimelineRawIntervalId::new(raw_id),
            ActivityInterval::try_new(
                TrackerRunId::from_bytes([8; 16]),
                range(start, end),
                state,
                evidence,
                None,
            )
            .expect("fixture activity satisfies invariants"),
        )
    }

    fn application_source(
        raw_id: u64,
        start: i64,
        end: i64,
        application_id: ApplicationId,
    ) -> TimelineSourceActivity {
        TimelineSourceActivity::new(
            TimelineRawIntervalId::new(raw_id),
            ActivityInterval::try_new(
                TrackerRunId::from_bytes([8; 16]),
                range(start, end),
                ActivityState::Active,
                ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
                    .expect("foreground evidence is valid"),
                Some(application_id),
            )
            .expect("fixture activity satisfies invariants"),
        )
    }

    #[test]
    fn powered_off_hover_keeps_exact_source_identity_and_times() {
        let activities = [source(22, 10, 20, ActivityState::PoweredOff)];
        let snapshot = TimelineProjector::build(
            TimelineContext::new(ProjectionContextKey::new(5), range(0, 30)),
            TimelineProjectionSource::new(DataRevision::new(2), &activities, &[]),
        )
        .expect("fixture snapshot builds");
        let detail = detail_at(&snapshot, TimelineBand::Activity, UtcMicros::new(12))
            .expect("powered-off interval is hit-testable despite its absent fill");
        assert_eq!(
            detail.value(),
            TimelineDetailValue::Activity(openmanic_application::ActivityStateValue::PoweredOff)
        );
        assert_eq!(
            detail.raw().expect("raw source is retained").raw_id(),
            TimelineRawIntervalId::new(22)
        );
        assert_eq!(
            detail.raw().expect("raw source is retained").raw_range(),
            range(10, 20)
        );
    }

    #[test]
    fn range_selection_is_a_deferred_today_action() {
        let selection = range(100, 120);
        let action = TimelineRenderAction::Today(TodayAction::SetTimelineSelection {
            selection: Some(selection),
        });
        assert_eq!(
            action,
            TimelineRenderAction::Today(TodayAction::SetTimelineSelection {
                selection: Some(selection),
            })
        );
    }

    #[test]
    fn hour_labels_cover_the_complete_midnight_to_midnight_axis() {
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(1), "1 AM");
        assert_eq!(hour_label(12), "12 PM");
        assert_eq!(hour_label(23), "11 PM");
        assert_eq!(hour_label(24), "12 AM");
    }

    #[test]
    fn selected_detail_times_use_the_day_relative_twelve_hour_clock() {
        let day = range(0, 24 * HOUR_US);
        assert_eq!(format_day_clock(day, UtcMicros::new(0)), "12:00 AM");
        assert_eq!(
            format_day_clock(day, UtcMicros::new(12 * HOUR_US)),
            "12:00 PM"
        );
        assert_eq!(
            format_day_clock(day, UtcMicros::new(24 * HOUR_US)),
            "12:00 AM"
        );
    }

    #[test]
    fn initial_view_uses_session_start_when_no_application_has_been_tracked() {
        let day = range(0, 24 * HOUR_US);
        let snapshot = TimelineProjector::build(
            TimelineContext::new(ProjectionContextKey::new(5), day),
            TimelineProjectionSource::new(DataRevision::new(2), &[], &[]),
        )
        .expect("empty fixture snapshot builds");

        assert_eq!(
            initial_view_range(&snapshot, UtcMicros::new(9 * HOUR_US)),
            range(9 * HOUR_US, 13 * HOUR_US)
        );
    }

    #[test]
    fn initial_view_grows_from_first_tracked_application_through_recent_activity() {
        let day = range(0, 24 * HOUR_US);
        let application_id = ApplicationId::from_bytes([3; 16]);
        let activities = [
            application_source(1, 8 * HOUR_US, 9 * HOUR_US, application_id),
            application_source(2, 12 * HOUR_US, 13 * HOUR_US, application_id),
        ];
        let applications = [TimelineApplication::new(application_id, None)];
        let snapshot = TimelineProjector::build(
            TimelineContext::new(ProjectionContextKey::new(5), day),
            TimelineProjectionSource::new(DataRevision::new(2), &activities, &applications),
        )
        .expect("tracked fixture snapshot builds");

        assert_eq!(
            initial_view_range(&snapshot, UtcMicros::new(10 * HOUR_US)),
            range(8 * HOUR_US, 14 * HOUR_US)
        );
    }

    #[test]
    fn overview_viewport_maps_and_pans_inside_the_complete_day() {
        let day = range(0, 24 * HOUR_US);
        let view = range(6 * HOUR_US, 12 * HOUR_US);
        let overview = Rect::from_min_max(pos2(0.0, 0.0), pos2(240.0, 14.0));
        let viewport = overview_view_rect(overview, day, view).expect("viewport maps into day");
        assert!((viewport.left() - 60.0).abs() < f32::EPSILON);
        assert!((viewport.right() - 120.0).abs() < f32::EPSILON);

        let panned = overview_range_for_pointer(
            overview,
            day,
            OverviewCapture::Pan {
                initial: view,
                grab_offset_us: i128::from(3 * HOUR_US),
            },
            150.0,
        )
        .expect("navigator pan remains a valid range");
        assert_eq!(panned, range(12 * HOUR_US, 18 * HOUR_US));
    }

    #[test]
    fn overview_hover_distinguishes_live_window_body_edges_and_empty_track() {
        let day = range(0, 24 * HOUR_US);
        let view = range(6 * HOUR_US, 12 * HOUR_US);
        let overview = Rect::from_min_max(pos2(0.0, 0.0), pos2(240.0, 24.0));

        assert_eq!(
            overview_hover_at_pointer(pos2(90.0, 12.0), overview, day, view),
            Some(OverviewHover::Body)
        );
        assert_eq!(
            overview_hover_at_pointer(pos2(60.0, 12.0), overview, day, view),
            Some(OverviewHover::StartEdge)
        );
        assert_eq!(
            overview_hover_at_pointer(pos2(120.0, 12.0), overview, day, view),
            Some(OverviewHover::EndEdge)
        );
        assert_eq!(
            overview_hover_at_pointer(pos2(200.0, 12.0), overview, day, view),
            None
        );
    }
}
