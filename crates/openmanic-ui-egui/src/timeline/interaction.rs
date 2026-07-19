//! Pure gesture arbitration for the three timeline bands.
//!
//! A caller supplies a single shared [`TimelineTransform`] for every visible band. This module
//! then produces time-based intents only; it neither selects a product record nor creates a
//! schedule. Record selection remains an exact index lookup through [`super::hit_test()`].

use openmanic_domain::{HalfOpenInterval, UtcMicros};

use super::TimelineTransform;

/// The logical movement required before a primary gesture is a drag rather than a click.
pub const DRAG_THRESHOLD_LOGICAL_PX: f32 = 4.0;

/// One input event already scoped to the timeline's allocated region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimelineGesture {
    /// Starts a primary gesture. Space changes it to horizontal panning.
    PrimaryPressed {
        /// Pointer x coordinate in the shared transform's coordinate space.
        x: f32,
        /// Whether Space was held at press time.
        space_held: bool,
        /// Whether the shell selected the explicitly labeled schedule-creation mode.
        create_schedule_mode: bool,
    },
    /// Starts a middle-button pan gesture.
    MiddlePressed {
        /// Pointer x coordinate in the shared transform's coordinate space.
        x: f32,
    },
    /// Updates the active primary or middle capture.
    PointerMoved {
        /// Pointer x coordinate in the shared transform's coordinate space.
        x: f32,
    },
    /// Ends a primary gesture.
    PrimaryReleased {
        /// Pointer x coordinate in the shared transform's coordinate space.
        x: f32,
    },
    /// Ends a middle-button pan gesture.
    MiddleReleased {
        /// Pointer x coordinate in the shared transform's coordinate space.
        x: f32,
    },
    /// Applies wheel or precision-scroll input over the timeline.
    Scrolled {
        /// Pointer x coordinate used as the zoom anchor.
        x: f32,
        /// Horizontal precision-scroll delta in logical pixels.
        horizontal_delta: f32,
        /// Vertical wheel delta. Positive values zoom in.
        vertical_delta: f32,
        /// Shift changes vertical wheel input into horizontal panning.
        shift_held: bool,
    },
    /// Restores the caller-supplied default timeline range.
    ResetView,
    /// Cancels a captured gesture or transient hover/popup state.
    Escape,
}

/// The one interaction response produced for a timeline input event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineInteractionResponse {
    view_range: HalfOpenInterval,
    event: Option<TimelineGestureEvent>,
    is_capturing: bool,
}

impl TimelineInteractionResponse {
    fn new(
        view_range: HalfOpenInterval,
        event: Option<TimelineGestureEvent>,
        is_capturing: bool,
    ) -> Self {
        Self {
            view_range,
            event,
            is_capturing,
        }
    }

    /// Returns the range that every band and overlay must use for the next shared transform.
    #[must_use]
    pub const fn view_range(self) -> HalfOpenInterval {
        self.view_range
    }

    /// Returns the optional action-only gesture intent.
    #[must_use]
    pub const fn event(self) -> Option<TimelineGestureEvent> {
        self.event
    }

    /// Returns whether the timeline retained pointer capture after this event.
    #[must_use]
    pub const fn is_capturing(self) -> bool {
        self.is_capturing
    }
}

/// An action-only result of timeline gesture arbitration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimelineGestureEvent {
    /// A click at an exact instant. The renderer resolves raw IDs with `hit_test` before selecting.
    Clicked {
        /// Exact UTC instant corresponding to the release point.
        instant: UtcMicros,
    },
    /// An in-progress range selection preview.
    RangePreview {
        /// Current exact half-open range.
        range: HalfOpenInterval,
    },
    /// A completed normal-mode range selection.
    RangeSelected {
        /// Exact half-open range selected by the gesture.
        range: HalfOpenInterval,
    },
    /// An in-progress, non-mutating schedule bracket preview.
    SchedulePreview {
        /// Current exact half-open provisional range.
        range: HalfOpenInterval,
    },
    /// A completed provisional range for the shared schedule editor to receive.
    ScheduleRequested {
        /// Exact half-open provisional range.
        range: HalfOpenInterval,
    },
    /// A pan, zoom, or reset changed the shared visible range.
    ViewChanged {
        /// New range for every timeline band and overlay.
        range: HalfOpenInterval,
    },
    /// Escape cancelled a capture or requests transient hover/popup clearing.
    Cancelled,
}

/// Retained pure interaction state for one timeline widget instance.
#[derive(Clone, Copy, Debug)]
pub struct TimelineInteraction {
    default_range: HalfOpenInterval,
    view_range: HalfOpenInterval,
    capture: Option<Capture>,
}

impl TimelineInteraction {
    /// Starts an interaction model at the default visible range.
    #[must_use]
    pub const fn new(default_range: HalfOpenInterval) -> Self {
        Self {
            default_range,
            view_range: default_range,
            capture: None,
        }
    }

    /// Returns the current range used to construct the one transform shared by all bands.
    #[must_use]
    pub const fn view_range(self) -> HalfOpenInterval {
        self.view_range
    }

    /// Returns whether a primary or middle gesture owns capture.
    #[must_use]
    pub const fn is_capturing(self) -> bool {
        self.capture.is_some()
    }

    /// Processes one timeline-scoped event without accessing storage or mutating product state.
    #[must_use]
    pub fn respond(
        &mut self,
        transform: TimelineTransform,
        gesture: TimelineGesture,
    ) -> TimelineInteractionResponse {
        self.view_range = transform.visible_range();
        let event = match gesture {
            TimelineGesture::PrimaryPressed {
                x,
                space_held,
                create_schedule_mode,
            } => self.primary_pressed(transform, x, space_held, create_schedule_mode),
            TimelineGesture::MiddlePressed { x } => self.middle_pressed(transform, x),
            TimelineGesture::PointerMoved { x } => self.pointer_moved(transform, x),
            TimelineGesture::PrimaryReleased { x } => self.primary_released(transform, x),
            TimelineGesture::MiddleReleased { x } => self.middle_released(transform, x),
            TimelineGesture::Scrolled {
                x,
                horizontal_delta,
                vertical_delta,
                shift_held,
            } => self.scrolled(transform, x, horizontal_delta, vertical_delta, shift_held),
            TimelineGesture::ResetView => self.reset_view(),
            TimelineGesture::Escape => Some(self.escape()),
        };
        TimelineInteractionResponse::new(self.view_range, event, self.is_capturing())
    }

    fn primary_pressed(
        &mut self,
        transform: TimelineTransform,
        x: f32,
        space_held: bool,
        create_schedule_mode: bool,
    ) -> Option<TimelineGestureEvent> {
        let instant = clamped_time_at_x(transform, x)?;
        self.capture = Some(if space_held {
            Capture::Pan(PanCapture::new(x, transform.visible_range()))
        } else {
            Capture::Primary(PrimaryCapture::new(instant, x, create_schedule_mode))
        });
        None
    }

    fn middle_pressed(
        &mut self,
        transform: TimelineTransform,
        x: f32,
    ) -> Option<TimelineGestureEvent> {
        if !x.is_finite() {
            return None;
        }
        self.capture = Some(Capture::Pan(PanCapture::new(x, transform.visible_range())));
        None
    }

    fn pointer_moved(
        &mut self,
        transform: TimelineTransform,
        x: f32,
    ) -> Option<TimelineGestureEvent> {
        match self.capture {
            Some(Capture::Pan(capture)) => self.pan_from_capture(transform, capture, x),
            Some(Capture::Primary(mut capture)) => {
                capture.dragged |= distance_exceeds_threshold(capture.start_x, x);
                self.capture = Some(Capture::Primary(capture));
                let range = capture_range(transform, capture.start_instant, x)?;
                if !capture.dragged {
                    return None;
                }
                Some(if capture.create_schedule_mode {
                    TimelineGestureEvent::SchedulePreview { range }
                } else {
                    TimelineGestureEvent::RangePreview { range }
                })
            }
            None => None,
        }
    }

    fn primary_released(
        &mut self,
        transform: TimelineTransform,
        x: f32,
    ) -> Option<TimelineGestureEvent> {
        let Capture::Primary(mut capture) = self.capture? else {
            return None;
        };
        capture.dragged |= distance_exceeds_threshold(capture.start_x, x);
        self.capture = None;
        if !capture.dragged {
            return clamped_time_at_x(transform, x)
                .map(|instant| TimelineGestureEvent::Clicked { instant });
        }
        let range = capture_range(transform, capture.start_instant, x)?;
        Some(if capture.create_schedule_mode {
            TimelineGestureEvent::ScheduleRequested { range }
        } else {
            TimelineGestureEvent::RangeSelected { range }
        })
    }

    fn middle_released(
        &mut self,
        transform: TimelineTransform,
        x: f32,
    ) -> Option<TimelineGestureEvent> {
        let Capture::Pan(capture) = self.capture? else {
            return None;
        };
        self.capture = None;
        self.pan_from_capture(transform, capture, x)
    }

    fn pan_from_capture(
        &mut self,
        transform: TimelineTransform,
        capture: PanCapture,
        x: f32,
    ) -> Option<TimelineGestureEvent> {
        let range = pan_range(transform, capture.start_range, x - capture.start_x)?;
        self.view_range = range;
        Some(TimelineGestureEvent::ViewChanged { range })
    }

    fn scrolled(
        &mut self,
        transform: TimelineTransform,
        x: f32,
        horizontal_delta: f32,
        vertical_delta: f32,
        shift_held: bool,
    ) -> Option<TimelineGestureEvent> {
        if !horizontal_delta.is_finite() || !vertical_delta.is_finite() {
            return None;
        }
        let range = if shift_held || horizontal_delta != 0.0 {
            pan_range(transform, transform.visible_range(), -horizontal_delta)?
        } else if vertical_delta != 0.0 {
            zoom_range(transform, x, vertical_delta)?
        } else {
            return None;
        };
        self.view_range = range;
        Some(TimelineGestureEvent::ViewChanged { range })
    }

    fn reset_view(&mut self) -> Option<TimelineGestureEvent> {
        self.capture = None;
        if self.view_range == self.default_range {
            return None;
        }
        self.view_range = self.default_range;
        Some(TimelineGestureEvent::ViewChanged {
            range: self.view_range,
        })
    }

    fn escape(&mut self) -> TimelineGestureEvent {
        self.capture = None;
        TimelineGestureEvent::Cancelled
    }
}

#[derive(Clone, Copy, Debug)]
enum Capture {
    Primary(PrimaryCapture),
    Pan(PanCapture),
}

#[derive(Clone, Copy, Debug)]
struct PrimaryCapture {
    start_instant: UtcMicros,
    start_x: f32,
    create_schedule_mode: bool,
    dragged: bool,
}

impl PrimaryCapture {
    const fn new(start_instant: UtcMicros, start_x: f32, create_schedule_mode: bool) -> Self {
        Self {
            start_instant,
            start_x,
            create_schedule_mode,
            dragged: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PanCapture {
    start_x: f32,
    start_range: HalfOpenInterval,
}

impl PanCapture {
    const fn new(start_x: f32, start_range: HalfOpenInterval) -> Self {
        Self {
            start_x,
            start_range,
        }
    }
}

fn distance_exceeds_threshold(start_x: f32, current_x: f32) -> bool {
    start_x.is_finite()
        && current_x.is_finite()
        && (current_x - start_x).abs() > DRAG_THRESHOLD_LOGICAL_PX
}

fn clamped_time_at_x(transform: TimelineTransform, x: f32) -> Option<UtcMicros> {
    if !x.is_finite() {
        return None;
    }
    transform.time_at_x(x.clamp(transform.x_start(), transform.x_end().next_down()))
}

fn capture_range(
    transform: TimelineTransform,
    start: UtcMicros,
    current_x: f32,
) -> Option<HalfOpenInterval> {
    let current = clamped_time_at_x(transform, current_x)?;
    let (start, end_inclusive) = if start <= current {
        (start, current)
    } else {
        (current, start)
    };
    let end = end_inclusive.get().checked_add(1).map(UtcMicros::new)?;
    HalfOpenInterval::try_new(start, end).ok()
}

fn pan_range(
    transform: TimelineTransform,
    source_range: HalfOpenInterval,
    pointer_delta: f32,
) -> Option<HalfOpenInterval> {
    if !pointer_delta.is_finite() {
        return None;
    }
    let shift = duration_for_pixels(source_range.duration_us(), pointer_delta, transform.width())?;
    shift_range(source_range, -shift)
}

fn zoom_range(
    transform: TimelineTransform,
    x: f32,
    vertical_delta: f32,
) -> Option<HalfOpenInterval> {
    let anchor = clamped_time_at_x(transform, x)?;
    let current = transform.visible_range();
    let factor = 2.0_f64.powf((-f64::from(vertical_delta) / 240.0).clamp(-3.0, 3.0));
    let desired_duration = scaled_duration(current.duration_us(), factor);
    anchored_range(current, anchor, desired_duration)
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "pixel input is converted only to an approximate, bounded view-navigation duration"
)]
fn duration_for_pixels(duration_us: u64, pixels: f32, width: f32) -> Option<i128> {
    if !width.is_finite() || width <= 0.0 {
        return None;
    }
    let delta = f64::from(pixels) * duration_us as f64 / f64::from(width);
    if !delta.is_finite() {
        return None;
    }
    Some(delta.round().clamp(i128::MIN as f64, i128::MAX as f64) as i128)
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the scale is finite and explicitly clamped to the positive u64 duration domain"
)]
fn scaled_duration(duration: u64, factor: f64) -> u64 {
    let scaled = (duration as f64 * factor).round();
    scaled.clamp(1.0, u64::MAX as f64) as u64
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "continuous zoom uses f64 ratios, then rounds down to a valid microsecond offset"
)]
fn anchored_range(
    current: HalfOpenInterval,
    anchor: UtcMicros,
    desired_duration: u64,
) -> Option<HalfOpenInterval> {
    let elapsed = i128::from(anchor.get()) - i128::from(current.start().get());
    let anchor_ratio = elapsed as f64 / current.duration_us() as f64;
    let new_elapsed = (anchor_ratio * desired_duration as f64).floor() as i128;
    range_from_start_and_duration(i128::from(anchor.get()) - new_elapsed, desired_duration)
}

fn shift_range(source: HalfOpenInterval, shift: i128) -> Option<HalfOpenInterval> {
    range_from_start_and_duration(
        i128::from(source.start().get()).saturating_add(shift),
        source.duration_us(),
    )
}

fn range_from_start_and_duration(start: i128, duration: u64) -> Option<HalfOpenInterval> {
    let min_start = i128::from(i64::MIN);
    let max_start = i128::from(i64::MAX) - i128::from(duration);
    let start = start.clamp(min_start, max_start);
    let end = start.checked_add(i128::from(duration))?;
    HalfOpenInterval::try_new(
        UtcMicros::new(i64::try_from(start).ok()?),
        UtcMicros::new(i64::try_from(end).ok()?),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{HalfOpenInterval, UtcMicros};

    use super::{
        DRAG_THRESHOLD_LOGICAL_PX, TimelineGesture, TimelineGestureEvent, TimelineInteraction,
    };
    use crate::timeline::TimelineTransform;

    fn range(start: i64, end: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
            .expect("positive fixture range")
    }

    fn transform(range: HalfOpenInterval) -> TimelineTransform {
        TimelineTransform::try_new(range, 0.0, 100.0).expect("valid fixture transform")
    }

    #[test]
    fn click_and_drag_are_arbitrated_by_the_strict_logical_threshold() {
        let initial_range = range(0, 1_000);
        let mut interaction = TimelineInteraction::new(initial_range);
        let transform = transform(initial_range);
        let _ = interaction.respond(
            transform,
            TimelineGesture::PrimaryPressed {
                x: 10.0,
                space_held: false,
                create_schedule_mode: false,
            },
        );
        let click = interaction.respond(
            transform,
            TimelineGesture::PrimaryReleased {
                x: 10.0 + DRAG_THRESHOLD_LOGICAL_PX,
            },
        );
        assert!(matches!(
            click.event(),
            Some(TimelineGestureEvent::Clicked { .. })
        ));

        let _ = interaction.respond(
            transform,
            TimelineGesture::PrimaryPressed {
                x: 10.0,
                space_held: false,
                create_schedule_mode: false,
            },
        );
        let selected = interaction.respond(
            transform,
            TimelineGesture::PrimaryReleased {
                x: 10.1 + DRAG_THRESHOLD_LOGICAL_PX,
            },
        );
        assert!(matches!(
            selected.event(),
            Some(TimelineGestureEvent::RangeSelected { range: selected_range })
                if selected_range == range(100, 142)
        ));
    }

    #[test]
    fn wheel_zoom_keeps_its_pointer_anchor_and_pan_moves_the_whole_view() {
        let initial_range = range(0, 1_000_000);
        let mut interaction = TimelineInteraction::new(initial_range);
        let transform = transform(initial_range);
        let zoomed = interaction.respond(
            transform,
            TimelineGesture::Scrolled {
                x: 25.0,
                horizontal_delta: 0.0,
                vertical_delta: 120.0,
                shift_held: false,
            },
        );
        let zoomed_range = zoomed.view_range();
        let zoomed_transform =
            TimelineTransform::try_new(zoomed_range, 0.0, 100.0).expect("valid zoomed transform");
        assert!(zoomed_range.duration_us() < initial_range.duration_us());
        assert_eq!(
            zoomed_transform.time_at_x(25.0),
            Some(UtcMicros::new(250_000)),
            "the pointer remains anchored to its original UTC instant"
        );

        let panned = interaction.respond(
            zoomed_transform,
            TimelineGesture::Scrolled {
                x: 50.0,
                horizontal_delta: 10.0,
                vertical_delta: 0.0,
                shift_held: false,
            },
        );
        assert_eq!(
            panned.view_range().duration_us(),
            zoomed_range.duration_us()
        );
        assert!(panned.view_range().start() > zoomed_range.start());
    }

    #[test]
    fn schedule_drag_and_escape_never_mutate_a_schedule() {
        let range = range(0, 1_000);
        let mut interaction = TimelineInteraction::new(range);
        let transform = transform(range);
        let _ = interaction.respond(
            transform,
            TimelineGesture::PrimaryPressed {
                x: 20.0,
                space_held: false,
                create_schedule_mode: true,
            },
        );
        let preview = interaction.respond(transform, TimelineGesture::PointerMoved { x: 50.0 });
        assert!(matches!(
            preview.event(),
            Some(TimelineGestureEvent::SchedulePreview { .. })
        ));
        let cancelled = interaction.respond(transform, TimelineGesture::Escape);
        assert_eq!(cancelled.event(), Some(TimelineGestureEvent::Cancelled));
        assert!(!cancelled.is_capturing());
    }

    #[test]
    fn reset_and_middle_capture_restore_and_report_the_shared_range() {
        let default = range(0, 1_000);
        let mut interaction = TimelineInteraction::new(default);
        let transform = transform(default);
        let _ = interaction.respond(transform, TimelineGesture::MiddlePressed { x: 50.0 });
        let panned = interaction.respond(transform, TimelineGesture::PointerMoved { x: 70.0 });
        assert_eq!(panned.view_range(), range(-200, 800));
        assert!(panned.is_capturing());
        let reset = interaction.respond(
            TimelineTransform::try_new(panned.view_range(), 0.0, 100.0)
                .expect("valid panned transform"),
            TimelineGesture::ResetView,
        );
        assert_eq!(reset.view_range(), default);
        assert!(!reset.is_capturing());
    }
}
