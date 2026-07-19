//! Pure OpenManic entities, value objects, state machines, invariants, and policies.
//!
//! This crate owns product facts and deliberately knows nothing about GUI, persistence,
//! operating-system APIs, or runtime channels. It has no threading or persistence assumptions.

#![forbid(unsafe_code)]

mod activity;
mod catalog;
mod ids;
mod time;

pub use activity::{
    ActivityCause, ActivityEvidence, ActivityEvidenceError, ActivityInterval,
    ActivityInvariantError, ActivityState, PowerTransitionEvidence, PowerTransitionEvidenceError,
};
pub use catalog::{
    Application, ApplicationError, ApplicationName, ApplicationNameKind, Category, CategoryName,
    CategoryNameKind, NameError, ValidatedName,
};
pub use ids::{
    ApplicationId, ApplicationIdKind, CategoryId, CategoryIdKind, OpaqueId, OpaqueIdParseError,
    TrackerRunId, TrackerRunIdKind,
};
pub use time::{HalfOpenInterval, HalfOpenIntervalError, UtcMicros, UtcMicrosArithmeticError};
