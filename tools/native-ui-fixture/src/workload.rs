//! Deterministic paint-only workload for the isolated native fixture.

use crate::{
    arguments::Arguments,
    report::{FrameSample, RunMeasurements, update_measurements},
};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use fixture_generator::ScenarioFixture;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SCRIPT_STEP_FRAMES: u32 = 30;
const CAMERA_SCALE: i64 = 1_000_000;
const MIN_CAMERA_SPAN: i64 = 50_000;
const POINTER_PAN_STEP: i64 = 80_000;
const PROJECTION_SCALE: u16 = 10_000;

/// Minimal eframe app that measures a deterministic synthetic paint workload.
pub(crate) struct NativeFixture {
    paint_items: Vec<PaintItem>,
    time_range: TimeRange,
    arguments: Arguments,
    launch_started: Instant,
    measurements: Arc<Mutex<RunMeasurements>>,
    frame_index: u32,
    last_update_started: Option<Instant>,
    camera: Camera,
}

impl NativeFixture {
    /// Builds an isolated workload from one frozen OM-030 fixture scenario.
    pub(crate) fn new(
        fixture: ScenarioFixture,
        arguments: Arguments,
        launch_started: Instant,
        measurements: Arc<Mutex<RunMeasurements>>,
    ) -> Self {
        let paint_items = PaintItem::from_fixture(fixture);
        let time_range = TimeRange::from_items(&paint_items);
        Self {
            paint_items,
            time_range,
            arguments,
            launch_started,
            measurements,
            frame_index: 0,
            last_update_started: None,
            camera: Camera::default(),
        }
    }

    fn paint_fixture(&mut self, ui: &mut egui::Ui) -> u64 {
        ui.horizontal(|ui| {
            ui.label("Diagnostic native fixture — not OpenManic product UI");
            ui.separator();
            ui.label(format!("{} paint records", self.paint_items.len()));
        });
        let available = ui.available_size_before_wrap();
        let desired = Vec2::new(available.x.max(240.0), available.y.max(220.0));
        let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
        self.apply_pointer_interaction(&response, rect);

        let paint_started = Instant::now();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_gray(22));
        let visible_range = self.camera.visible_range(self.time_range);
        for item in &self.paint_items {
            if !visible_range.overlaps(item.start, item.end) {
                continue;
            }
            paint_item(&painter, rect, visible_range, item, self.camera.selected_id);
        }
        duration_ns(paint_started.elapsed())
    }

    fn apply_pointer_interaction(&mut self, response: &egui::Response, rect: Rect) {
        if response.dragged() && rect.width() > 0.0 {
            let drag = response.drag_delta().x;
            if drag > 0.0 {
                self.camera.pan(-POINTER_PAN_STEP);
            } else if drag < 0.0 {
                self.camera.pan(POINTER_PAN_STEP);
            }
        }
        if response.double_clicked() {
            self.camera.reset();
        }
        if response.clicked() {
            self.camera.selected_id = self
                .paint_items
                .get((self.frame_index as usize) % self.paint_items.len().max(1))
                .map(|item| item.id);
        }
    }

    fn scripted_interaction(&mut self) -> &'static str {
        let action = match (self.frame_index / SCRIPT_STEP_FRAMES) % 5 {
            0 => ScriptedInteraction::ZoomIn,
            1 => ScriptedInteraction::PanForward,
            2 => ScriptedInteraction::SelectNext,
            3 => ScriptedInteraction::ZoomOut,
            _ => ScriptedInteraction::Reset,
        };
        action.apply(&mut self.camera, &self.paint_items, self.frame_index)
    }

    fn record_frame(
        &self,
        scripted_interaction: &'static str,
        ui_cpu_ns: u64,
        dense_paint_preparation_ns: u64,
        observed_frame_cadence_ns: Option<u64>,
    ) {
        if self.frame_index < self.arguments.warmup_frame_count {
            return;
        }
        update_measurements(&self.measurements, |measurements| {
            measurements.record_frame(FrameSample {
                frame_index: self.frame_index,
                scripted_interaction,
                ui_cpu_ns,
                dense_paint_preparation_ns,
                observed_frame_cadence_ns,
            });
        });
    }

    fn record_memory_checkpoint(&self, checkpoint: &'static str) {
        update_measurements(&self.measurements, |measurements| {
            measurements.record_memory_checkpoint(checkpoint, self.frame_index);
        });
    }
}

impl eframe::App for NativeFixture {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let update_started = Instant::now();
        let observed_frame_cadence_ns = self
            .last_update_started
            .map(|previous| duration_ns(update_started.duration_since(previous)));
        self.last_update_started = Some(update_started);

        let scripted_interaction = self.scripted_interaction();
        let dense_paint_preparation_ns = self.paint_fixture(ui);
        let ui_cpu_ns = duration_ns(update_started.elapsed());

        if self.frame_index == 0 {
            update_measurements(&self.measurements, |measurements| {
                measurements.record_shell_ready(duration_ns(self.launch_started.elapsed()));
            });
            self.record_memory_checkpoint("shell_ready");
        }
        if self.frame_index == self.arguments.warmup_frame_count {
            self.record_memory_checkpoint("idle_after_warmup");
        }
        if self.frame_index == self.arguments.warmup_frame_count.saturating_add(1) {
            self.record_memory_checkpoint("dense_interaction");
        }

        self.record_frame(
            scripted_interaction,
            ui_cpu_ns,
            dense_paint_preparation_ns,
            observed_frame_cadence_ns,
        );
        self.frame_index = self.frame_index.saturating_add(1);
        if self.frame_index >= self.arguments.frame_count {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ui.ctx().request_repaint();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PaintItem {
    id: u64,
    start: i64,
    end: i64,
    lane: usize,
}

impl PaintItem {
    fn from_fixture(fixture: ScenarioFixture) -> Vec<Self> {
        let ScenarioFixture {
            activity_intervals,
            category_band,
            state_band,
            application_band,
            schedules,
            overlays,
            ..
        } = fixture;
        let mut items = Vec::with_capacity(
            activity_intervals.len()
                + category_band.len()
                + state_band.len()
                + application_band.len()
                + schedules.len()
                + overlays.len(),
        );
        items.extend(activity_intervals.into_iter().map(|interval| Self {
            id: interval.id,
            start: interval.start.get(),
            end: interval.end.get(),
            lane: 0,
        }));
        items.extend(category_band.into_iter().map(|segment| Self {
            id: 10_000_000_u64 + segment.id,
            start: segment.start.get(),
            end: segment.end.get(),
            lane: 1,
        }));
        items.extend(state_band.into_iter().map(|segment| Self {
            id: 20_000_000_u64 + segment.id,
            start: segment.start.get(),
            end: segment.end.get(),
            lane: 2,
        }));
        items.extend(application_band.into_iter().map(|segment| Self {
            id: 30_000_000_u64 + segment.id,
            start: segment.start.get(),
            end: segment.end.get(),
            lane: 3,
        }));
        items.extend(schedules.into_iter().map(|schedule| Self {
            id: 40_000_000_u64 + schedule.id,
            start: schedule.start.get(),
            end: schedule.end.get(),
            lane: 3,
        }));
        items.extend(overlays.into_iter().map(|overlay| Self {
            id: 50_000_000_u64 + overlay.id,
            start: overlay.start.get(),
            end: overlay.end.get(),
            lane: 2,
        }));
        items.sort_unstable_by_key(|item| (item.start, item.end, item.id));
        items
    }
}

fn paint_item(
    painter: &egui::Painter,
    rect: Rect,
    range: TimeRange,
    item: &PaintItem,
    selected_id: Option<u64>,
) {
    let left = project(item.start, range, rect);
    let right = project(item.end, range, rect).max(left + 1.0);
    let lane_height = rect.height() / 4.0;
    let top = rect.top() + lane_height * lane_offset(item.lane) + 2.0;
    let bottom = top + (lane_height - 4.0).max(1.0);
    let color = if selected_id == Some(item.id) {
        Color32::from_rgb(245, 190, 65)
    } else {
        color_for(item.id, item.lane)
    };
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
        1.0,
        color,
    );
}

fn project(value: i64, range: TimeRange, rect: Rect) -> f32 {
    let duration = range.end.saturating_sub(range.start).max(1);
    let offset = value.saturating_sub(range.start).clamp(0, duration);
    let scaled = offset
        .saturating_mul(i64::from(PROJECTION_SCALE))
        .div_euclid(duration);
    let scaled = u16::try_from(scaled).unwrap_or(PROJECTION_SCALE);
    let normalized = f32::from(scaled) / f32::from(PROJECTION_SCALE);
    rect.left() + rect.width() * normalized
}

fn lane_offset(lane: usize) -> f32 {
    match lane {
        0 => 0.0,
        1 => 1.0,
        2 => 2.0,
        _ => 3.0,
    }
}

fn color_for(id: u64, lane: usize) -> Color32 {
    let channel = (id % 140) as u8;
    match lane {
        0 => Color32::from_rgb(75_u8.saturating_add(channel), 130, 205),
        1 => Color32::from_rgb(80, 150_u8.saturating_add(channel / 2), 110),
        2 => Color32::from_rgb(170, 95_u8.saturating_add(channel / 2), 85),
        _ => Color32::from_rgb(155, 100, 175_u8.saturating_add(channel / 2)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimeRange {
    start: i64,
    end: i64,
}

impl TimeRange {
    fn from_items(items: &[PaintItem]) -> Self {
        let Some(first) = items.first() else {
            return Self { start: 0, end: 1 };
        };
        let start = first.start;
        let end = items
            .iter()
            .map(|item| item.end)
            .max()
            .unwrap_or(first.end)
            .max(start.saturating_add(1));
        Self { start, end }
    }

    fn overlaps(self, start: i64, end: i64) -> bool {
        start < self.end && end > self.start
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Camera {
    center: i64,
    span: i64,
    selected_id: Option<u64>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: CAMERA_SCALE / 2,
            span: CAMERA_SCALE,
            selected_id: None,
        }
    }
}

impl Camera {
    fn visible_range(self, full: TimeRange) -> TimeRange {
        let total = full.end.saturating_sub(full.start).max(1);
        let span = total
            .saturating_mul(self.span)
            .div_euclid(CAMERA_SCALE)
            .max(1);
        let center = full
            .start
            .saturating_add(total.saturating_mul(self.center).div_euclid(CAMERA_SCALE));
        let start = center.saturating_sub(span / 2);
        let end = center.saturating_add(span / 2);
        TimeRange {
            start: start.max(full.start),
            end: end.min(full.end).max(full.start.saturating_add(1)),
        }
    }

    fn zoom(&mut self, numerator: i64, denominator: i64) {
        let denominator = denominator.max(1);
        self.span = self
            .span
            .saturating_mul(numerator)
            .div_euclid(denominator)
            .clamp(MIN_CAMERA_SPAN, CAMERA_SCALE);
    }

    fn pan(&mut self, delta: i64) {
        self.center = self.center.saturating_add(delta).clamp(0, CAMERA_SCALE);
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptedInteraction {
    ZoomIn,
    PanForward,
    SelectNext,
    ZoomOut,
    Reset,
}

impl ScriptedInteraction {
    fn apply(self, camera: &mut Camera, items: &[PaintItem], frame_index: u32) -> &'static str {
        match self {
            Self::ZoomIn => {
                camera.zoom(82, 100);
                "zoom_in"
            }
            Self::PanForward => {
                camera.pan(POINTER_PAN_STEP);
                "pan_forward"
            }
            Self::SelectNext => {
                camera.selected_id = items
                    .get((frame_index as usize) % items.len().max(1))
                    .map(|item| item.id);
                "select_next"
            }
            Self::ZoomOut => {
                camera.zoom(120, 100);
                "zoom_out"
            }
            Self::Reset => {
                camera.reset();
                "reset"
            }
        }
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{Camera, PaintItem, ScriptedInteraction, TimeRange};
    use fixture_generator::Scenario;

    #[test]
    fn dense_fixture_becomes_ten_thousand_paint_only_items() {
        let fixture = Scenario::Dense10000IntervalRange.generate(2_026_030);
        let items = PaintItem::from_fixture(fixture);
        assert_eq!(items.len(), 10_000);
        assert!(items.windows(2).all(|pair| pair[0].start <= pair[1].start));
    }

    #[test]
    fn representative_segmented_fixture_keeps_independent_bands() {
        let fixture = Scenario::ThreeSegmentedBands.generate(2_026_030);
        let items = PaintItem::from_fixture(fixture);
        assert_eq!(items.len(), 12);
        assert_eq!(items.iter().filter(|item| item.lane == 1).count(), 3);
        assert_eq!(items.iter().filter(|item| item.lane == 2).count(), 4);
        assert_eq!(items.iter().filter(|item| item.lane == 3).count(), 5);
    }

    #[test]
    fn scripted_interactions_repeat_without_pointer_input() {
        let mut camera = Camera::default();
        let items = [PaintItem {
            id: 1,
            start: 0,
            end: 1,
            lane: 0,
        }];
        assert_eq!(
            ScriptedInteraction::ZoomIn.apply(&mut camera, &items, 0),
            "zoom_in"
        );
        assert!(camera.span < 1_000_000);
        assert_eq!(
            ScriptedInteraction::SelectNext.apply(&mut camera, &items, 0),
            "select_next"
        );
        assert_eq!(camera.selected_id, Some(1));
    }

    #[test]
    fn visible_range_stays_valid_at_camera_edges() {
        let mut camera = Camera::default();
        camera.pan(-2_000_000);
        camera.zoom(5, 100);
        let visible = camera.visible_range(TimeRange {
            start: 10,
            end: 110,
        });
        assert!(visible.start >= 10);
        assert!(visible.end > visible.start);
    }
}
