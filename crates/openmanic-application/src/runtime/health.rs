//! Worker health states and policy-driven escalation decisions.

use crate::runtime::RuntimeWorker;

/// The terminal or recoverable failure class reported by a worker root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerFailure {
    /// The worker returned an expected-but-unrecoverable typed failure.
    ReturnedError,
    /// The worker panicked at its containment boundary.
    Panicked,
}

/// The health state visible to the runtime supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerHealthState {
    /// The named worker root has been configured but has not reported ready.
    Starting,
    /// The worker is available and has no outstanding failure state.
    Healthy,
    /// The worker is unavailable while a controlled recovery action is pending.
    Degraded {
        /// The observed failure that caused the degraded state.
        failure: WorkerFailure,
    },
    /// The worker stopped and may be recreated by its supervisor.
    Failed {
        /// The observed failure that stopped the worker.
        failure: WorkerFailure,
    },
    /// The runtime cannot safely continue the affected service.
    Fatal {
        /// The observed failure that requires fatal handling.
        failure: WorkerFailure,
    },
}

/// The required follow-up after a worker root reports a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerEscalation {
    /// Recreate a safe worker after closing its read-only resources.
    RestartWorker {
        /// The recoverable worker to recreate.
        worker: RuntimeWorker,
    },
    /// Mark platform attribution unavailable and schedule a controlled probe.
    RetryPlatformProbe,
    /// Enter storage-fatal mode without transparently restarting the writer.
    StorageFatal,
    /// End the process because supervision itself cannot guarantee coordination.
    SupervisorFatal,
}

/// Holds the current health of one named runtime worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerHealth {
    worker: RuntimeWorker,
    state: WorkerHealthState,
}

impl WorkerHealth {
    /// Creates an unstarted health record for the given worker.
    #[must_use]
    pub const fn new(worker: RuntimeWorker) -> Self {
        Self {
            worker,
            state: WorkerHealthState::Starting,
        }
    }

    /// Returns the worker whose health this record represents.
    #[must_use]
    pub const fn worker(self) -> RuntimeWorker {
        self.worker
    }

    /// Returns the current worker health state.
    #[must_use]
    pub const fn state(self) -> WorkerHealthState {
        self.state
    }

    /// Marks the worker ready after startup or a successful recovery.
    pub fn mark_healthy(&mut self) {
        self.state = WorkerHealthState::Healthy;
    }

    /// Records a worker failure and returns its required escalation policy.
    #[must_use]
    pub fn report_failure(&mut self, failure: WorkerFailure) -> WorkerEscalation {
        let escalation = escalation_for(self.worker);
        self.state = match escalation {
            WorkerEscalation::RestartWorker { .. } => WorkerHealthState::Failed { failure },
            WorkerEscalation::RetryPlatformProbe => WorkerHealthState::Degraded { failure },
            WorkerEscalation::StorageFatal | WorkerEscalation::SupervisorFatal => {
                WorkerHealthState::Fatal { failure }
            }
        };
        escalation
    }
}

const fn escalation_for(worker: RuntimeWorker) -> WorkerEscalation {
    match worker {
        RuntimeWorker::ProjectionReader | RuntimeWorker::BulkWorker => {
            WorkerEscalation::RestartWorker { worker }
        }
        RuntimeWorker::PlatformObservation => WorkerEscalation::RetryPlatformProbe,
        RuntimeWorker::Writer => WorkerEscalation::StorageFatal,
        RuntimeWorker::Supervisor => WorkerEscalation::SupervisorFatal,
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerEscalation, WorkerFailure, WorkerHealth, WorkerHealthState};
    use crate::runtime::RuntimeWorker;

    #[test]
    fn recoverable_workers_fail_with_an_explicit_restart_policy() {
        let mut health = WorkerHealth::new(RuntimeWorker::ProjectionReader);
        health.mark_healthy();

        assert_eq!(
            health.report_failure(WorkerFailure::Panicked),
            WorkerEscalation::RestartWorker {
                worker: RuntimeWorker::ProjectionReader
            }
        );
        assert_eq!(
            health.state(),
            WorkerHealthState::Failed {
                failure: WorkerFailure::Panicked
            }
        );
    }

    #[test]
    fn platform_failure_degrades_then_requests_a_controlled_probe() {
        let mut health = WorkerHealth::new(RuntimeWorker::PlatformObservation);

        assert_eq!(
            health.report_failure(WorkerFailure::ReturnedError),
            WorkerEscalation::RetryPlatformProbe
        );
        assert_eq!(
            health.state(),
            WorkerHealthState::Degraded {
                failure: WorkerFailure::ReturnedError
            }
        );
    }

    #[test]
    fn writer_failure_is_storage_fatal_and_never_requests_restart() {
        let mut health = WorkerHealth::new(RuntimeWorker::Writer);

        assert_eq!(
            health.report_failure(WorkerFailure::Panicked),
            WorkerEscalation::StorageFatal
        );
        assert_eq!(
            health.state(),
            WorkerHealthState::Fatal {
                failure: WorkerFailure::Panicked
            }
        );
    }
}
