//! Bounded, adapter-independent runtime coordination primitives.
//!
//! This module owns queue backpressure, latest-value coalescing, cancellation,
//! worker health, thread-root naming, and shutdown ordering. It deliberately
//! does not create threads, invoke storage, call platform APIs, or perform UI
//! work. Composition owns those effects and drives these deterministic types.

mod cancellation;
mod health;
mod lanes;
mod mailbox;
mod shutdown;
mod supervisor;

pub use cancellation::{CancellationRequest, CancellationSource, CancellationToken};
pub use health::{WorkerEscalation, WorkerFailure, WorkerHealth, WorkerHealthState};
pub use lanes::{
    LaneCapacities, LaneConfigurationError, LaneReceive, LaneRetentionReason, LaneSubmit,
    RuntimeLaneReceiver, RuntimeLanes, WorkLane, bounded_runtime_lanes,
};
pub use mailbox::{
    LatestMailbox, LatestMailboxReceiver, MailboxPublish, MailboxReceive, latest_mailbox,
};
pub use shutdown::{
    ShutdownAdvance, ShutdownCoordinator, ShutdownError, ShutdownPhase, ShutdownStep,
};
pub use supervisor::{RuntimeSupervisor, RuntimeWorker, ThreadRoot};
