//! Explicit, ordered graceful-shutdown coordination.

/// The required non-blocking coordination step for an explicit Quit request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownStep {
    /// Reject newly submitted nonessential commands and jobs.
    RejectNonessentialWork,
    /// Request cancellation at safe boundaries for reads and bulk work.
    CancelSafeReads,
    /// Finalize or checkpoint active tracking and focus state.
    CheckpointCriticalActivity,
    /// Flush pending atomic settings and layout updates.
    FlushSettings,
    /// Stop and join projection readers and bulk workers.
    JoinReadersAndWorkers,
    /// Checkpoint and close the single SQLite writer.
    CloseWriter,
    /// Stop platform observation and remove platform-owned resources.
    StopPlatform,
    /// Join the supervisor after every owned worker has stopped.
    JoinSupervisor,
}

/// Current phase of the explicit shutdown lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownPhase {
    /// Normal operation still accepts eligible work.
    Running,
    /// The coordinator is waiting for the stated ordered step to finish.
    Executing(ShutdownStep),
    /// A critical checkpoint, settings flush, or writer close failed.
    FlushFailed {
        /// The critical step that may be retried or explicitly skipped.
        step: ShutdownStep,
    },
    /// All ordered shutdown steps have completed or an explicit Quit Anyway skipped a failure.
    Complete,
}

/// Reports a lifecycle transition after an ordered shutdown operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownAdvance {
    /// Explicit Quit began and nonessential work must now be rejected.
    Begun(ShutdownStep),
    /// The current step completed and this is the next required step.
    Next(ShutdownStep),
    /// A critical flush failed and requires Retry or Quit Anyway.
    FlushFailed(ShutdownStep),
    /// The sequence is complete and the UI may exit.
    Complete,
}

/// Rejects an invalid shutdown lifecycle transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownError {
    /// Explicit Quit was requested after shutdown had already started.
    AlreadyStarted,
    /// A completion or failure did not match the currently executing step.
    StepOutOfOrder {
        /// The coordinator's current step, when one exists.
        expected: Option<ShutdownStep>,
        /// The step reported by the caller.
        received: ShutdownStep,
    },
    /// A failure was reported for a non-critical step that cannot enter flush recovery.
    NonCriticalFailure(ShutdownStep),
    /// Retry or Quit Anyway was requested while no critical failure was pending.
    NoFlushFailure,
}

/// Drives graceful-shutdown ordering without performing any blocking work itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownCoordinator {
    phase: ShutdownPhase,
}

impl ShutdownCoordinator {
    /// Creates a running coordinator that has not received an explicit Quit request.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: ShutdownPhase::Running,
        }
    }

    /// Returns the current shutdown lifecycle phase.
    #[must_use]
    pub const fn phase(self) -> ShutdownPhase {
        self.phase
    }

    /// Returns whether new nonessential work may be accepted.
    #[must_use]
    pub const fn accepts_nonessential_work(self) -> bool {
        matches!(self.phase, ShutdownPhase::Running)
    }

    /// Starts explicit shutdown by rejecting future nonessential work.
    ///
    /// # Errors
    ///
    /// Returns [`ShutdownError::AlreadyStarted`] when shutdown has already begun.
    pub fn begin(&mut self) -> Result<ShutdownAdvance, ShutdownError> {
        if !matches!(self.phase, ShutdownPhase::Running) {
            return Err(ShutdownError::AlreadyStarted);
        }

        self.phase = ShutdownPhase::Executing(ShutdownStep::RejectNonessentialWork);
        Ok(ShutdownAdvance::Begun(ShutdownStep::RejectNonessentialWork))
    }

    /// Completes the current ordered step and returns the next required action.
    ///
    /// # Errors
    ///
    /// Returns [`ShutdownError::StepOutOfOrder`] when the reported step is not current.
    pub fn complete(&mut self, step: ShutdownStep) -> Result<ShutdownAdvance, ShutdownError> {
        self.require_current_step(step)?;
        if let Some(next) = next_step(step) {
            self.phase = ShutdownPhase::Executing(next);
            Ok(ShutdownAdvance::Next(next))
        } else {
            self.phase = ShutdownPhase::Complete;
            Ok(ShutdownAdvance::Complete)
        }
    }

    /// Records a critical checkpoint, settings, or writer-close failure.
    ///
    /// # Errors
    ///
    /// Returns [`ShutdownError::StepOutOfOrder`] for a non-current step and
    /// [`ShutdownError::NonCriticalFailure`] for a step that cannot be retried.
    pub fn fail_critical(&mut self, step: ShutdownStep) -> Result<ShutdownAdvance, ShutdownError> {
        self.require_current_step(step)?;
        if !is_critical_flush(step) {
            return Err(ShutdownError::NonCriticalFailure(step));
        }

        self.phase = ShutdownPhase::FlushFailed { step };
        Ok(ShutdownAdvance::FlushFailed(step))
    }

    /// Retries the critical step that most recently failed.
    ///
    /// # Errors
    ///
    /// Returns [`ShutdownError::NoFlushFailure`] when no critical failure is pending.
    pub fn retry_critical_flush(&mut self) -> Result<ShutdownAdvance, ShutdownError> {
        let ShutdownPhase::FlushFailed { step } = self.phase else {
            return Err(ShutdownError::NoFlushFailure);
        };
        self.phase = ShutdownPhase::Executing(step);
        Ok(ShutdownAdvance::Next(step))
    }

    /// Explicitly skips a failed critical flush and continues the remaining joins.
    ///
    /// This transition does not claim that the failed critical operation succeeded.
    ///
    /// # Errors
    ///
    /// Returns [`ShutdownError::NoFlushFailure`] when no critical failure is pending.
    pub fn quit_anyway(&mut self) -> Result<ShutdownAdvance, ShutdownError> {
        let ShutdownPhase::FlushFailed { step } = self.phase else {
            return Err(ShutdownError::NoFlushFailure);
        };
        if let Some(next) = next_step(step) {
            self.phase = ShutdownPhase::Executing(next);
            Ok(ShutdownAdvance::Next(next))
        } else {
            self.phase = ShutdownPhase::Complete;
            Ok(ShutdownAdvance::Complete)
        }
    }

    fn require_current_step(self, received: ShutdownStep) -> Result<(), ShutdownError> {
        let expected = match self.phase {
            ShutdownPhase::Executing(step) => Some(step),
            ShutdownPhase::Running
            | ShutdownPhase::FlushFailed { .. }
            | ShutdownPhase::Complete => None,
        };
        if expected == Some(received) {
            Ok(())
        } else {
            Err(ShutdownError::StepOutOfOrder { expected, received })
        }
    }
}

const fn next_step(step: ShutdownStep) -> Option<ShutdownStep> {
    match step {
        ShutdownStep::RejectNonessentialWork => Some(ShutdownStep::CancelSafeReads),
        ShutdownStep::CancelSafeReads => Some(ShutdownStep::CheckpointCriticalActivity),
        ShutdownStep::CheckpointCriticalActivity => Some(ShutdownStep::FlushSettings),
        ShutdownStep::FlushSettings => Some(ShutdownStep::JoinReadersAndWorkers),
        ShutdownStep::JoinReadersAndWorkers => Some(ShutdownStep::CloseWriter),
        ShutdownStep::CloseWriter => Some(ShutdownStep::StopPlatform),
        ShutdownStep::StopPlatform => Some(ShutdownStep::JoinSupervisor),
        ShutdownStep::JoinSupervisor => None,
    }
}

const fn is_critical_flush(step: ShutdownStep) -> bool {
    matches!(
        step,
        ShutdownStep::CheckpointCriticalActivity
            | ShutdownStep::FlushSettings
            | ShutdownStep::CloseWriter
    )
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ShutdownAdvance, ShutdownCoordinator, ShutdownError, ShutdownPhase, ShutdownStep};

    #[test]
    fn shutdown_runs_the_required_join_order_and_rejects_nonessential_work_first() {
        let mut shutdown = ShutdownCoordinator::new();
        let ordered_steps = [
            ShutdownStep::RejectNonessentialWork,
            ShutdownStep::CancelSafeReads,
            ShutdownStep::CheckpointCriticalActivity,
            ShutdownStep::FlushSettings,
            ShutdownStep::JoinReadersAndWorkers,
            ShutdownStep::CloseWriter,
            ShutdownStep::StopPlatform,
            ShutdownStep::JoinSupervisor,
        ];

        assert!(shutdown.accepts_nonessential_work());
        assert_eq!(
            shutdown.begin(),
            Ok(ShutdownAdvance::Begun(ShutdownStep::RejectNonessentialWork))
        );
        assert!(!shutdown.accepts_nonessential_work());

        for (index, step) in ordered_steps.iter().copied().enumerate() {
            let expected = ordered_steps
                .get(index + 1)
                .copied()
                .map_or(ShutdownAdvance::Complete, ShutdownAdvance::Next);
            assert_eq!(shutdown.complete(step), Ok(expected));
        }

        assert_eq!(shutdown.phase(), ShutdownPhase::Complete);
    }

    #[test]
    fn critical_failure_requires_explicit_retry_or_quit_anyway() {
        let mut shutdown = ShutdownCoordinator::new();
        shutdown
            .begin()
            .expect("a running coordinator accepts its first Quit request");
        shutdown
            .complete(ShutdownStep::RejectNonessentialWork)
            .expect("rejecting nonessential work advances to cancellation");
        shutdown
            .complete(ShutdownStep::CancelSafeReads)
            .expect("safe reads cancel before critical checkpointing");

        assert_eq!(
            shutdown.fail_critical(ShutdownStep::CheckpointCriticalActivity),
            Ok(ShutdownAdvance::FlushFailed(
                ShutdownStep::CheckpointCriticalActivity
            ))
        );
        assert_eq!(
            shutdown.retry_critical_flush(),
            Ok(ShutdownAdvance::Next(
                ShutdownStep::CheckpointCriticalActivity
            ))
        );
        assert_eq!(
            shutdown.fail_critical(ShutdownStep::CheckpointCriticalActivity),
            Ok(ShutdownAdvance::FlushFailed(
                ShutdownStep::CheckpointCriticalActivity
            ))
        );
        assert_eq!(
            shutdown.quit_anyway(),
            Ok(ShutdownAdvance::Next(ShutdownStep::FlushSettings))
        );
    }

    #[test]
    fn shutdown_rejects_out_of_order_or_noncritical_failure_reports() {
        let mut shutdown = ShutdownCoordinator::new();
        assert_eq!(
            shutdown.complete(ShutdownStep::CloseWriter),
            Err(ShutdownError::StepOutOfOrder {
                expected: None,
                received: ShutdownStep::CloseWriter,
            })
        );
        shutdown
            .begin()
            .expect("a running coordinator accepts its first Quit request");
        assert_eq!(
            shutdown.fail_critical(ShutdownStep::RejectNonessentialWork),
            Err(ShutdownError::NonCriticalFailure(
                ShutdownStep::RejectNonessentialWork
            ))
        );
    }
}
