//! Paint-only three-band timeline rendering and UI-local detail state.
//!
//! The renderer consumes an immutable [`TimelineSnapshot`] and one [`PresentableData`] state. It
//! makes exactly one egui interaction allocation for the timeline canvas, draws interval geometry
//! through the OM-282 paint plan, and returns actions for its owner to reduce. It does not access
//! ports, clone history, or mutate canonical category/application data.

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Painter, PointerButton, Pos2, Rect, Response, Sense,
    Stroke, Ui, Vec2,
};
use openmanic_application::{
    ActivityStateValue, ApplicationBandValue, CategoryBandValue, DataCompleteness, TimelineSnapshot,
};
use openmanic_domain::{ActivityState, ApplicationId, HalfOpenInterval};

use super::{
    PaintFill, PaintPrimitive, TimelineBand, TimelineDetail, TimelineDetailValue, TimelineGesture,
    TimelineGestureEvent, TimelineInteraction, TimelinePaintPlan, TimelineTransform, hit_test,
    prepare_schedule_overlays,
};
use crate::{DataLimitation, PresentableData, TodayAction, TodayViewContext};

const BAND_LABEL_WIDTH: f32 = 108.0;
const BAND_HEIGHT: f32 = 26.0;
const BAND_GAP: f32 = 6.0;
const TICK_HEIGHT: f32 = 22.0;
const TIMELINE_HEIGHT: f32 = TICK_HEIGHT + (BAND_HEIGHT * 3.0) + (BAND_GAP * 2.0);
const DETAIL_WIDTH: f32 = 320.0;
const DETAIL_LINE_HEIGHT: f32 = 16.0;

const CANVAS: Color32 = Color32::from_rgb(18, 22, 31);
const BAND_BACKGROUND: Color32 = Color32::from_rgb(37, 45, 61);
const BAND_BORDER: Color32 = Color32::from_rgb(99, 114, 137);
const LABEL: Color32 = Color32::from_rgb(222, 230, 241);
const MUTED: Color32 = Color32::from_rgb(170, 184, 204);
const SELECTED: Color32 = Color32::from_rgb(255, 255, 255);
const HOVER_BACKGROUND: Color32 = Color32::from_rgb(27, 33, 45);
const ERROR: Color32 = Color32::from_rgb(237, 113, 113);
const WARNING: Color32 = Color32::from_rgb(236, 190, 93);
const SUCCESS: Color32 = Color32::from_rgb(107, 201, 139);
const SCHEDULE_BRACKET: Color32 = Color32::from_rgb(133, 201, 255);

/// An action emitted by the timeline renderer for its owning controller to reduce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineRenderAction {
    /// Applies a UI-local Today action after the frame; selection itself emits no mutation action.
    Today(TodayAction),
    /// Requests a replacement immutable projection for a changed pan or zoom range.
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
#[derive(Debug, Default)]
pub struct TimelineRenderer {
    interaction: Option<TimelineInteraction>,
    persistent_detail: Option<TimelineDetail>,
    primary_down: bool,
    middle_down: bool,
    pressed_band: Option<TimelineBand>,
    last_pointer: Option<Pos2>,
    snapshot_range: Option<HalfOpenInterval>,
}

impl TimelineRenderer {
    /// Creates an empty renderer that initializes itself from its first immutable snapshot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            interaction: None,
            persistent_detail: None,
            primary_down: false,
            middle_down: false,
            pressed_band: None,
            last_pointer: None,
            snapshot_range: None,
        }
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
    pub fn show_snapshot(
        &mut self,
        ui: &mut Ui,
        snapshot: &TimelineSnapshot,
        context: &TodayViewContext,
        create_schedule_mode: bool,
    ) -> TimelineRenderOutput {
        self.ensure_interaction(snapshot.visible_range());
        let (canvas, response) = ui.allocate_exact_size(
            Vec2::new(
                ui.available_width().max(BAND_LABEL_WIDTH + 1.0),
                TIMELINE_HEIGHT,
            ),
            Sense::click_and_drag(),
        );
        let timeline_rect = Rect::from_min_max(
            Pos2::new(canvas.min.x + BAND_LABEL_WIDTH, canvas.min.y + TICK_HEIGHT),
            canvas.max,
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
        self.paint_snapshot(
            ui.painter(),
            canvas,
            snapshot,
            transform,
            band_rects,
            context,
        );

        let mut output = TimelineRenderOutput::default();
        if let Some(gesture) = self.next_gesture(ui, &response, band_rects, create_schedule_mode) {
            self.apply_gesture(snapshot, transform, gesture, &mut output);
        }
        if !self.primary_down && !self.middle_down {
            output.hover_detail = self.hover_detail(snapshot, transform, band_rects);
        }
        if let (Some(detail), Some(pointer)) = (output.hover_detail, self.last_pointer) {
            paint_hover_detail(ui.painter(), ui.clip_rect(), pointer, detail);
        }
        self.paint_persistent_detail(ui, &mut output);
        output.persistent_detail = self.persistent_detail;
        output
    }

    fn ensure_interaction(&mut self, default_range: HalfOpenInterval) {
        let should_reset = self.snapshot_range != Some(default_range) || self.interaction.is_none();
        if should_reset {
            self.interaction = Some(TimelineInteraction::new(default_range));
            self.snapshot_range = Some(default_range);
        }
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
        if input.keys.escape_pressed {
            self.primary_down = false;
            self.middle_down = false;
            self.pressed_band = None;
            return Some(TimelineGesture::Escape);
        }
        if input.buttons.primary_down && !self.primary_down && response.hovered() {
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
        if input.buttons.middle_down && !self.middle_down && response.hovered() {
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
        if response.hovered() && (input.scroll.x != 0.0 || input.scroll.y != 0.0) {
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
                output
                    .actions
                    .push(TimelineRenderAction::ViewRangeChanged { range });
                if !contains_range(snapshot.visible_range(), range) {
                    self.interaction = Some(TimelineInteraction::new(snapshot.visible_range()));
                    self.snapshot_range = Some(snapshot.visible_range());
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
        canvas: Rect,
        snapshot: &TimelineSnapshot,
        transform: TimelineTransform,
        band_rects: BandRects,
        context: &TodayViewContext,
    ) {
        painter.rect_filled(canvas, 4.0, CANVAS);
        paint_band_labels(painter, band_rects);
        paint_ticks(painter, canvas, transform);
        let plan = TimelinePaintPlan::from_snapshot(transform, snapshot);
        paint_category_band(painter, plan.category(), band_rects.category);
        paint_activity_band(painter, plan.activity(), band_rects.activity);
        paint_application_band(painter, plan.application(), band_rects.application);
        paint_schedule_overlays(painter, transform, band_rects, snapshot);
        paint_selection(painter, transform, band_rects, context.timeline_selection());
        paint_persistent_selection(painter, transform, band_rects, self.persistent_detail);
        paint_completeness(painter, canvas, snapshot.completeness());
    }

    fn paint_persistent_detail(&self, ui: &mut Ui, output: &mut TimelineRenderOutput) {
        let Some(detail) = self.persistent_detail else {
            return;
        };
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong("Selected timeline detail");
            render_detail_lines(ui, detail);
            if let (Some(label), Some(action)) = (detail.value().action_label(), detail.action())
                && ui.button(label).clicked()
            {
                output.actions.push(TimelineRenderAction::Today(action));
            }
            if let TimelineDetailValue::Application(ApplicationBandValue::Application(
                application_id,
            )) = detail.value()
                && ui.button("Edit this application in Categories").clicked()
            {
                output
                    .actions
                    .push(TimelineRenderAction::OpenCategories { application_id });
            }
        });
    }
}

fn paint_schedule_overlays(
    painter: &Painter,
    transform: TimelineTransform,
    band_rects: BandRects,
    snapshot: &TimelineSnapshot,
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
        let stroke = Stroke::new(if adjusted { 2.0 } else { 1.0 }, SCHEDULE_BRACKET);
        painter.line_segment([Pos2::new(pixels.start_x(), top), Pos2::new(pixels.end_x(), top)], stroke);
        painter.line_segment([Pos2::new(pixels.start_x(), top), Pos2::new(pixels.start_x(), bottom)], stroke);
        painter.line_segment([Pos2::new(pixels.end_x(), top), Pos2::new(pixels.end_x(), bottom)], stroke);
    }
}

#[derive(Clone, Copy, Debug)]
struct BandRects {
    category: Rect,
    activity: Rect,
    application: Rect,
}

impl BandRects {
    fn new(timeline_rect: Rect) -> Self {
        let category = Rect::from_min_size(
            timeline_rect.min,
            Vec2::new(timeline_rect.width(), BAND_HEIGHT),
        );
        let activity = Rect::from_min_size(
            Pos2::new(category.min.x, category.max.y + BAND_GAP),
            Vec2::new(timeline_rect.width(), BAND_HEIGHT),
        );
        let application = Rect::from_min_size(
            Pos2::new(activity.min.x, activity.max.y + BAND_GAP),
            Vec2::new(timeline_rect.width(), BAND_HEIGHT),
        );
        Self {
            category,
            activity,
            application,
        }
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

fn paint_band_labels(painter: &Painter, bands: BandRects) {
    for (band, rect) in [
        (TimelineBand::Category, bands.category),
        (TimelineBand::Activity, bands.activity),
        (TimelineBand::Application, bands.application),
    ] {
        painter.text(
            Pos2::new(rect.min.x - BAND_LABEL_WIDTH + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            band.label(),
            FontId::proportional(12.0),
            LABEL,
        );
        painter.rect_filled(rect, 2.0, BAND_BACKGROUND);
        painter.line_segment(
            [rect.left_top(), rect.right_top()],
            Stroke::new(1.0, BAND_BORDER),
        );
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, BAND_BORDER),
        );
    }
}

fn paint_ticks(painter: &Painter, canvas: Rect, transform: TimelineTransform) {
    let Ok(layout) = super::AdaptiveTickLayout::try_new(56.0, 8.0) else {
        return;
    };
    for tick in layout.generate(transform).ticks() {
        painter.line_segment(
            [
                Pos2::new(tick.x(), canvas.min.y + TICK_HEIGHT - 5.0),
                Pos2::new(tick.x(), canvas.min.y + TICK_HEIGHT),
            ],
            Stroke::new(1.0, MUTED),
        );
        painter.text(
            Pos2::new(tick.x(), canvas.min.y + 2.0),
            Align2::CENTER_TOP,
            format!("UTC {}", tick.instant().get()),
            FontId::monospace(10.0),
            MUTED,
        );
    }
}

fn paint_category_band(
    painter: &Painter,
    band: &super::PaintBand<'_, CategoryBandValue>,
    rect: Rect,
) {
    paint_band(painter, band, rect, |value| match value {
        CategoryBandValue::Category(category_id) => identity_color(category_id.as_bytes()),
        CategoryBandValue::Uncategorized => Color32::from_rgb(190, 143, 92),
        CategoryBandValue::NonApplicationState(state) => activity_color(*state),
    });
}

fn paint_activity_band(
    painter: &Painter,
    band: &super::PaintBand<'_, ActivityStateValue>,
    rect: Rect,
) {
    paint_band(painter, band, rect, |value| activity_color(value.state()));
}

fn paint_application_band(
    painter: &Painter,
    band: &super::PaintBand<'_, ApplicationBandValue>,
    rect: Rect,
) {
    paint_band(painter, band, rect, |value| match value {
        ApplicationBandValue::Application(application_id) => {
            identity_color(application_id.as_bytes())
        }
        ApplicationBandValue::NoApplication(state) => activity_color(*state),
        ApplicationBandValue::UnresolvedApplication => Color32::from_rgb(150, 111, 184),
    });
}

fn paint_band<T>(
    painter: &Painter,
    band: &super::PaintBand<'_, T>,
    rect: Rect,
    color_for: impl Fn(&T) -> Color32,
) {
    for primitive in band.primitives() {
        match primitive {
            PaintPrimitive::Segment(segment) => {
                paint_primitive(
                    painter,
                    rect,
                    segment.geometry().pixels(),
                    segment.fill(),
                    color_for(segment.interval().value()),
                );
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
                paint_primitive(painter, rect, bin.pixels(), PaintFill::Visible, color);
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
) {
    let rect = Rect::from_min_max(
        Pos2::new(pixels.start_x(), band_rect.min.y + 1.0),
        Pos2::new(
            pixels.end_x().max(pixels.start_x() + 1.0),
            band_rect.max.y - 1.0,
        ),
    );
    if fill == PaintFill::Visible {
        painter.rect_filled(rect, 0.0, color);
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
}

fn paint_selection(
    painter: &Painter,
    transform: TimelineTransform,
    bands: BandRects,
    selection: Option<HalfOpenInterval>,
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
        0.0,
        Color32::from_rgba_unmultiplied(121, 151, 255, 50),
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
    let top = Pos2::new(geometry.pixels().start_x(), band.min.y + 1.0);
    let bottom = Pos2::new(geometry.pixels().end_x(), band.max.y - 1.0);
    painter.line_segment(
        [top, Pos2::new(top.x, bottom.y)],
        Stroke::new(1.5, SELECTED),
    );
    painter.line_segment(
        [Pos2::new(bottom.x, top.y), bottom],
        Stroke::new(1.5, SELECTED),
    );
}

fn paint_completeness(painter: &Painter, canvas: Rect, completeness: DataCompleteness) {
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

fn paint_hover_detail(canvas_painter: &Painter, clip: Rect, pointer: Pos2, detail: TimelineDetail) {
    let text = detail_text(detail);
    let height = (4.0 * DETAIL_LINE_HEIGHT) + 12.0;
    let x = if pointer.x + 16.0 + DETAIL_WIDTH <= clip.max.x {
        pointer.x + 16.0
    } else {
        (pointer.x - DETAIL_WIDTH - 16.0).max(clip.min.x)
    };
    let y = (pointer.y + 16.0).min(clip.max.y - height).max(clip.min.y);
    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(DETAIL_WIDTH, height));
    canvas_painter.rect_filled(rect, 4.0, HOVER_BACKGROUND);
    canvas_painter.text(
        rect.min + Vec2::new(6.0, 6.0),
        Align2::LEFT_TOP,
        text,
        FontId::monospace(11.0),
        LABEL,
    );
}

fn render_detail_lines(ui: &mut Ui, detail: TimelineDetail) {
    ui.label(format!("Band: {}", detail.band().label()));
    ui.label(format!("Value: {}", detail.value().label()));
    ui.label(format!("Interval: {}", range_text(detail.interval_range())));
    if let Some(raw) = detail.raw() {
        ui.label(format!("Raw interval: {}", raw.raw_id().get()));
        ui.label(format!("Raw time: {}", range_text(raw.raw_range())));
    } else {
        ui.label("Source: explicit unknown activity gap");
    }
}

fn detail_text(detail: TimelineDetail) -> String {
    match detail.raw() {
        Some(raw) => format!(
            "{}\n{}\nInterval: {}\nRaw {}: {}",
            detail.band().label(),
            detail.value().label(),
            range_text(detail.interval_range()),
            raw.raw_id().get(),
            range_text(raw.raw_range()),
        ),
        None => format!(
            "{}\n{}\nInterval: {}\nSource: explicit unknown activity gap",
            detail.band().label(),
            detail.value().label(),
            range_text(detail.interval_range()),
        ),
    }
}

fn range_text(range: HalfOpenInterval) -> String {
    format!("{} µs – {} µs", range.start().get(), range.end().get())
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
    ui.painter().rect_filled(rect, 4.0, CANVAS);
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
        ActivityState::Active => Color32::from_rgb(99, 185, 129),
        ActivityState::Idle => Color32::from_rgb(220, 176, 83),
        ActivityState::PausedByUser => Color32::from_rgb(108, 145, 207),
        ActivityState::Excluded => Color32::from_rgb(145, 111, 169),
        ActivityState::Unavailable => Color32::from_rgb(201, 104, 104),
        ActivityState::PoweredOff => Color32::TRANSPARENT,
        ActivityState::UnknownMissing => Color32::from_rgb(128, 128, 128),
    }
}

const fn identity_color(bytes: [u8; 16]) -> Color32 {
    Color32::from_rgb(
        72 + (bytes[0] % 128),
        72 + (bytes[5] % 128),
        72 + (bytes[10] % 128),
    )
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

    use super::{TimelineBand, TimelineDetailValue, TimelineRenderAction, detail_at};
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
}
