//! Minimal adapter ports consumed by application use cases.

use crate::{
    ApplicationError, CommandEnvelope, CommandReceipt, DataRevision, ProjectionRequest,
    SnapshotEnvelope, TrackingPersistenceIntent,
};

/// Submits typed commands without exposing queue, storage, platform, or UI types.
pub trait CommandPort<P> {
    /// Accepts a command for lossless application handling.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError`] when the command boundary cannot accept the
    /// command according to its explicit availability or backpressure policy.
    fn submit(&mut self, command: CommandEnvelope<P>) -> Result<CommandReceipt, ApplicationError>;
}

/// Produces immutable presentation-ready snapshots without exposing storage details.
pub trait ProjectionPort<K, V> {
    /// Produces a snapshot for a typed projection request.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError`] when the projection boundary cannot produce
    /// a snapshot according to its explicit availability or failure policy.
    fn project(
        &mut self,
        request: ProjectionRequest<K>,
    ) -> Result<SnapshotEnvelope<V>, ApplicationError>;
}

/// Persists one complete tracking transition or checkpoint without exposing storage details.
///
/// The composition root supplies a bounded writer-backed implementation. It must either
/// commit the entire intent or return it unchanged to the caller; partial persistence is not
/// an allowed result. The trait is synchronous and fakeable so the reducer has no SQLite,
/// thread, channel, or platform dependency.
pub trait TrackingPersistencePort {
    /// Attempts one atomic persistence submission without blocking the caller.
    #[must_use]
    fn try_persist(&mut self, intent: TrackingPersistenceIntent) -> TrackingPersistenceSubmit;
}

/// The explicit result of submitting a tracking persistence intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrackingPersistenceSubmit {
    /// The intent committed atomically at this monotonic data revision.
    Committed(DataRevision),
    /// A bounded writer could not accept the intent, which remains owned by the caller.
    Retained {
        /// The unchanged intent available for deterministic retry or shutdown handling.
        intent: TrackingPersistenceIntent,
        /// Why a bounded persistence boundary retained the intent.
        reason: TrackingPersistenceRetentionReason,
    },
    /// The persistence implementation failed without committing the intent.
    Failed(ApplicationError),
}

/// Explains why a tracking persistence boundary retained authoritative work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingPersistenceRetentionReason {
    /// The bounded writer lane reached capacity.
    QueueFull,
    /// The writer has stopped accepting new work.
    WriterClosed,
}
