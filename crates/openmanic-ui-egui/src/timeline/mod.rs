//! Pure geometry, hit testing, and tick preparation for the activity timeline.
//!
//! These helpers deliberately have no egui responses, storage access, or background work. The
//! paint and interaction tasks consume the same [`TimelineTransform`] for every timeline layer.

pub mod detail;
pub mod geometry;
pub mod hit_test;
pub mod interaction;
pub mod paint;
pub mod renderer;
pub mod ticks;

pub use detail::{
    TimelineBand, TimelineDetail, TimelineDetailValue, TimelineRawDetail, TimelineScheduleDetail,
};

pub use geometry::{
    BandSegmentGeometry, PixelRange, ScheduleBracketGeometry, TimelineRangeGeometry,
    TimelineTransform, TimelineTransformError, band_geometry,
};
pub use hit_test::{TimelineHit, hit_test};
pub use interaction::{
    TimelineGesture, TimelineGestureEvent, TimelineInteraction, TimelineInteractionResponse,
};
pub use paint::{
    ActivityPaintBand, PaintBand, PaintFill, PaintPrimitive, ScheduleOverlayGeometry,
    TimelinePaintPlan, prepare_schedule_overlays,
};
pub use renderer::{TimelineRenderAction, TimelineRenderOutput, TimelineRenderer};
pub use ticks::{AdaptiveTickLayout, TickGeneration, TickLayoutError, TimelineTick};
