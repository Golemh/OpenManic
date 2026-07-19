//! Minimal adapter ports consumed by application use cases.

use crate::{
    ApplicationError, CommandEnvelope, CommandReceipt, ProjectionRequest, SnapshotEnvelope,
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
