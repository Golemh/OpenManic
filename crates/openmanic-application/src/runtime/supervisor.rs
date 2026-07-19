//! Named runtime roots and deterministic supervisor health tracking.

use std::collections::BTreeMap;

use crate::runtime::{WorkerEscalation, WorkerFailure, WorkerHealth};

/// Identifies a named thread root managed by the application runtime.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeWorker {
    /// Coordinates runtime lifecycle and worker health.
    Supervisor,
    /// Owns the single SQLite writer connection.
    Writer,
    /// Performs short SQLite reads and projection work.
    ProjectionReader,
    /// Performs cancellable bulk background work.
    BulkWorker,
    /// Observes normalized platform activity evidence.
    PlatformObservation,
}

/// Names one thread root without creating or owning the actual thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadRoot {
    worker: RuntimeWorker,
}

impl ThreadRoot {
    /// Creates a root representation for a worker whose thread is composed elsewhere.
    #[must_use]
    pub const fn new(worker: RuntimeWorker) -> Self {
        Self { worker }
    }

    /// Returns the worker assigned to this root.
    #[must_use]
    pub const fn worker(self) -> RuntimeWorker {
        self.worker
    }

    /// Returns the stable diagnostic-safe thread name for this root.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self.worker {
            RuntimeWorker::Supervisor => "openmanic-supervisor",
            RuntimeWorker::Writer => "openmanic-writer",
            RuntimeWorker::ProjectionReader => "openmanic-projection-reader",
            RuntimeWorker::BulkWorker => "openmanic-bulk-worker",
            RuntimeWorker::PlatformObservation => "openmanic-platform-observation",
        }
    }
}

/// Maintains health records and policy decisions for named worker roots.
#[derive(Debug)]
pub struct RuntimeSupervisor {
    health: BTreeMap<RuntimeWorker, WorkerHealth>,
}

impl RuntimeSupervisor {
    /// Creates a supervisor with one starting health record per named root.
    #[must_use]
    pub fn new(roots: impl IntoIterator<Item = ThreadRoot>) -> Self {
        let health = roots
            .into_iter()
            .map(|root| (root.worker(), WorkerHealth::new(root.worker())))
            .collect();
        Self { health }
    }

    /// Returns the current health for a registered worker.
    #[must_use]
    pub fn health(&self, worker: RuntimeWorker) -> Option<WorkerHealth> {
        self.health.get(&worker).copied()
    }

    /// Marks a registered worker healthy after startup or controlled recovery.
    ///
    /// Returns `false` when the worker root was not registered with this supervisor.
    pub fn mark_healthy(&mut self, worker: RuntimeWorker) -> bool {
        let Some(health) = self.health.get_mut(&worker) else {
            return false;
        };
        health.mark_healthy();
        true
    }

    /// Records a registered worker failure and returns its escalation decision.
    ///
    /// Returns `None` when the caller reports an unregistered worker.
    #[must_use]
    pub fn report_failure(
        &mut self,
        worker: RuntimeWorker,
        failure: WorkerFailure,
    ) -> Option<WorkerEscalation> {
        self.health
            .get_mut(&worker)
            .map(|health| health.report_failure(failure))
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeSupervisor, RuntimeWorker, ThreadRoot};
    use crate::runtime::{WorkerEscalation, WorkerFailure, WorkerHealthState};

    #[test]
    fn thread_roots_have_stable_named_worker_boundaries() {
        assert_eq!(
            ThreadRoot::new(RuntimeWorker::Writer).name(),
            "openmanic-writer"
        );
        assert_eq!(
            ThreadRoot::new(RuntimeWorker::PlatformObservation).name(),
            "openmanic-platform-observation"
        );
    }

    #[test]
    fn supervisor_tracks_registered_health_and_escalates_failures() {
        let mut supervisor = RuntimeSupervisor::new([
            ThreadRoot::new(RuntimeWorker::ProjectionReader),
            ThreadRoot::new(RuntimeWorker::Writer),
        ]);

        assert!(supervisor.mark_healthy(RuntimeWorker::ProjectionReader));
        assert_eq!(
            supervisor.report_failure(RuntimeWorker::ProjectionReader, WorkerFailure::Panicked),
            Some(WorkerEscalation::RestartWorker {
                worker: RuntimeWorker::ProjectionReader
            })
        );
        assert_eq!(
            supervisor
                .health(RuntimeWorker::ProjectionReader)
                .expect("registered worker retains a health record")
                .state(),
            WorkerHealthState::Failed {
                failure: WorkerFailure::Panicked
            }
        );
        assert_eq!(
            supervisor.report_failure(RuntimeWorker::BulkWorker, WorkerFailure::ReturnedError),
            None
        );
    }
}
