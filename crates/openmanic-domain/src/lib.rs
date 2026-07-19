//! Pure OpenManic entities, value objects, state machines, invariants, and policies.
//!
//! This crate owns product facts and deliberately knows nothing about GUI, persistence,
//! operating-system APIs, or runtime channels. It has no threading or persistence assumptions.

#![forbid(unsafe_code)]

mod activity;
mod catalog;
mod documents;
mod focus;
mod ids;
mod schedule;
mod time;

pub use activity::{
    ActivityCause, ActivityEvidence, ActivityEvidenceError, ActivityInterval,
    ActivityInvariantError, ActivityState, PowerTransitionEvidence, PowerTransitionEvidenceError,
};
pub use catalog::{
    Application, ApplicationError, ApplicationName, ApplicationNameKind, Category, CategoryName,
    CategoryNameKind, NameError, ValidatedName,
};
pub use documents::{LayoutDocument, SavedViewDocument, SettingsDocument, ThemeSelection};
pub use focus::{FocusSession, FocusSessionError, FocusSessionState};
pub use ids::{
    ApplicationId, ApplicationIdKind, CategoryId, CategoryIdKind, FocusSessionId,
    FocusSessionIdKind, OneTimeScheduleId, OneTimeScheduleIdKind, OpaqueId, OpaqueIdParseError,
    ScheduleSeriesId, ScheduleSeriesIdKind, TrackerRunId, TrackerRunIdKind,
};
pub use schedule::{ScheduleEditScope, ScheduleRule, ScheduleValidationError};
pub use time::{HalfOpenInterval, HalfOpenIntervalError, UtcMicros, UtcMicrosArithmeticError};
