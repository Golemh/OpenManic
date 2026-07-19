//! Pure geometry, hit testing, and tick preparation for the activity timeline.
//!
//! These helpers deliberately have no egui responses, storage access, or background work. The
//! paint and interaction tasks consume the same [`TimelineTransform`] for every timeline layer.

pub mod geometry;
pub mod hit_test;
pub mod ticks;

pub use geometry::{
    BandSegmentGeometry, PixelRange, ScheduleBracketGeometry, TimelineRangeGeometry,
    TimelineTransform, TimelineTransformError, band_geometry,
};
pub use hit_test::{TimelineHit, hit_test};
pub use ticks::{AdaptiveTickLayout, TickGeneration, TickLayoutError, TimelineTick};
