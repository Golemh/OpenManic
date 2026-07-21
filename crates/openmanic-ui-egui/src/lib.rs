//! egui/eframe presentation for immutable OpenManic application snapshots.
//!
//! This crate owns GUI lifecycle and rendering. It emits typed actions and deliberately cannot
//! depend on concrete storage or platform adapters. Frame updates must not block on I/O, platform
//! calls, recurrence expansion, or full-history aggregation.

#![forbid(unsafe_code)]

#[cfg(all(feature = "renderer-wgpu", feature = "renderer-glow"))]
compile_error!("select exactly one renderer: renderer-wgpu or renderer-glow");

#[cfg(not(any(feature = "renderer-wgpu", feature = "renderer-glow")))]
compile_error!("select one renderer: renderer-wgpu or renderer-glow");

mod app;
mod calendar;
mod controller;
pub mod design;
mod distribution;
mod jobs;
mod layout;
mod layout_editor;
mod model;
mod overview;
mod reducer;
mod repaint;
mod settings;
mod shell;
mod shutdown;
mod theme;
pub mod timeline;
mod today;
mod usage;

pub use app::OpenManicApp;
pub use calendar::{
    CalendarAction, CalendarBlockDetails, CalendarBlockKind, CalendarController, CalendarDataState,
    CalendarDensePeriod, CalendarEffect, CalendarPresentedBlock, CalendarViewModel,
};
pub use controller::{
    CommandDispatcher, DispatchDrain, InboundMessage, QueueCapacityError, QueueOverflow,
    UiController,
};
pub use distribution::{
    DistributionBuildError, DistributionContribution, DistributionGrouping, DistributionSnapshot,
    render_distribution_snapshot,
};
pub use jobs::{
    DestructiveConfirmation, JobDescriptor, JobPresentation, JobPresentationState, JobProgress,
    JobsAction, JobsController, JobsEffect, JobsViewModel,
};
pub use layout::{DashboardColumnCount, DashboardPlacement, DashboardReflow, reflow_dashboard};
pub use layout_editor::{LayoutEditAction, LayoutEditEffect, LayoutEditor};
pub use model::{
    DataLimitation, EmptyReason, MutationStatus, PresentableData, Route, RouteLocalState,
    SnapshotReception, TodayCategoryFilter, TodayNarrowingCriterion, TodayViewContext, UiAction,
    UiModel, UserFacingError,
};
pub use overview::{
    OverviewAction, OverviewController, OverviewDataState, OverviewEffect,
    OverviewPresentedAllocation, OverviewSavedViewItem, OverviewViewModel,
};
pub use settings::{
    SettingsAction, SettingsAdvancedDraft, SettingsBasicDraft, SettingsController, SettingsEffect,
    TITLE_COLLECTION_DISCLOSURE,
};
pub use shutdown::{
    ShutdownAction, ShutdownController, ShutdownEffect, ShutdownFailureViewModel,
    render_shutdown_failure,
};
pub use theme::{
    BuiltInThemeMode, ResolvedTheme, ThemeController, ThemeResolutionError, ThemeTokens,
};
pub use timeline::{
    AdaptiveTickLayout, BandSegmentGeometry, PixelRange, ScheduleBracketGeometry, TickGeneration,
    TickLayoutError, TimelineHit, TimelineRangeGeometry, TimelineTick, TimelineTransform,
    TimelineTransformError, band_geometry, hit_test,
};
pub use today::{
    TodayAction, TodayController, TodayTrackingRequest, TodayWidgetBinding, TodayWidgetBindings,
    TodayWidgetDefinition, TodayWidgetInstance, TodayWidgetInstanceId, TodayWidgetKind,
    TodayWidgetRegistry, TodayWidgetRegistryError, TodayWidgetResolution, TodayWidgetSizePolicy,
    TrackingControlAcknowledgement, TrackingControlAction,
};
pub use usage::{ApplicationUsage, ApplicationUsageSnapshot, render_usage_snapshot};
