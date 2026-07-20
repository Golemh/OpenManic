//! Reusable presentation state for concurrent background jobs and recoverable failures.
//!
//! This module deliberately owns only local interaction state. Composition supplies named jobs,
//! authoritative lifecycle updates, and dispatches the resulting effects.

use std::collections::BTreeMap;

use openmanic_application::{JobId, JobState};

/// A named background job known to the current UI session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobDescriptor {
    job_id: JobId,
    name: String,
    scope: String,
}

impl JobDescriptor {
    /// Creates a named job with a user-visible statement of its affected scope.
    #[must_use]
    pub fn new(job_id: JobId, name: String, scope: String) -> Self {
        Self {
            job_id,
            name,
            scope,
        }
    }

    /// Returns the stable application job identity.
    #[must_use]
    pub const fn job_id(&self) -> JobId {
        self.job_id
    }

    /// Returns the user-visible operation name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the scope affected by the operation.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }
}

/// Optional non-authoritative progress information for a running job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobProgress {
    completed: u64,
    total: Option<u64>,
    activity: Option<String>,
}

impl JobProgress {
    /// Creates progress with an optional total unit count and activity description.
    #[must_use]
    pub fn new(completed: u64, total: Option<u64>, activity: Option<String>) -> Self {
        Self {
            completed,
            total,
            activity,
        }
    }

    /// Returns the completed work-unit count.
    #[must_use]
    pub const fn completed(&self) -> u64 {
        self.completed
    }

    /// Returns the known total work-unit count, when one exists.
    #[must_use]
    pub const fn total(&self) -> Option<u64> {
        self.total
    }

    /// Returns the current safe, user-visible activity description.
    #[must_use]
    pub fn activity(&self) -> Option<&str> {
        self.activity.as_deref()
    }
}

/// Render-ready lifecycle state for one job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobPresentationState {
    /// The job is waiting to start.
    Queued,
    /// The job is executing.
    Running,
    /// Cancellation is pending at the next safe boundary.
    Cancelling,
    /// The job finished successfully.
    Succeeded,
    /// The job ended after honoring cancellation.
    Cancelled,
    /// The job failed and should remain discoverable until dismissed or retried.
    Failed {
        /// A safe stable failure description.
        message: String,
    },
    /// The job stopped before an authoritative terminal result was known.
    Interrupted,
}

impl JobPresentationState {
    /// Converts the immutable application lifecycle contract into render-ready state.
    #[must_use]
    pub fn from_job_state(state: &JobState) -> Self {
        match state {
            JobState::Queued => Self::Queued,
            JobState::Running => Self::Running,
            JobState::Cancelling => Self::Cancelling,
            JobState::Succeeded => Self::Succeeded,
            JobState::Cancelled => Self::Cancelled,
            JobState::Failed { error } => Self::Failed {
                message: error.to_string(),
            },
            JobState::Interrupted => Self::Interrupted,
        }
    }

    /// Returns whether cancellation can still be requested.
    #[must_use]
    pub const fn can_cancel(&self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    /// Returns whether retry is a valid recoverable affordance.
    #[must_use]
    pub const fn can_retry(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::Interrupted)
    }

    /// Returns whether this completed item may be dismissed from local presentation.
    #[must_use]
    pub const fn can_dismiss(&self) -> bool {
        !matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }
}

/// One render-ready row in the reusable concurrent-job component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobPresentation {
    descriptor: JobDescriptor,
    state: JobPresentationState,
    progress: Option<JobProgress>,
}

impl JobPresentation {
    /// Returns the stable job identity.
    #[must_use]
    pub const fn job_id(&self) -> JobId {
        self.descriptor.job_id()
    }

    /// Returns the named operation.
    #[must_use]
    pub fn name(&self) -> &str {
        self.descriptor.name()
    }

    /// Returns the operation scope.
    #[must_use]
    pub fn scope(&self) -> &str {
        self.descriptor.scope()
    }

    /// Returns the render-ready lifecycle state.
    #[must_use]
    pub const fn state(&self) -> &JobPresentationState {
        &self.state
    }

    /// Returns optional progress/activity information.
    #[must_use]
    pub const fn progress(&self) -> Option<&JobProgress> {
        self.progress.as_ref()
    }
}

/// Explicit confirmation content for an action that may destroy or replace user data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestructiveConfirmation {
    action_id: String,
    title: String,
    scope: String,
    confirm_label: String,
}

impl DestructiveConfirmation {
    /// Creates visible scope and wording for one explicit destructive action.
    #[must_use]
    pub fn new(action_id: String, title: String, scope: String, confirm_label: String) -> Self {
        Self {
            action_id,
            title,
            scope,
            confirm_label,
        }
    }

    /// Returns the composition-owned action identity.
    #[must_use]
    pub fn action_id(&self) -> &str {
        &self.action_id
    }

    /// Returns the confirmation heading.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the data or configuration scope that will be affected.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Returns the explicit label for the irreversible action.
    #[must_use]
    pub fn confirm_label(&self) -> &str {
        &self.confirm_label
    }
}

/// Complete render input for the reusable jobs/errors component.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JobsViewModel {
    jobs: Vec<JobPresentation>,
    destructive_confirmation: Option<DestructiveConfirmation>,
}

impl JobsViewModel {
    /// Returns deterministically job-ID-sorted concurrent job rows.
    #[must_use]
    pub fn jobs(&self) -> &[JobPresentation] {
        &self.jobs
    }

    /// Returns the outstanding explicit destructive confirmation, when any.
    #[must_use]
    pub const fn destructive_confirmation(&self) -> Option<&DestructiveConfirmation> {
        self.destructive_confirmation.as_ref()
    }
}

/// Local interaction with the reusable jobs/errors component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobsAction {
    /// Requests cancellation at the job's next safe boundary.
    RequestCancel {
        /// Identifies the active job to cancel.
        job_id: JobId,
    },
    /// Requests a new execution for a recoverably failed or interrupted job.
    Retry {
        /// Identifies the recoverable job to retry.
        job_id: JobId,
    },
    /// Hides a completed job from this UI session.
    Dismiss {
        /// Identifies the terminal job presentation to hide.
        job_id: JobId,
    },
    /// Opens destructive confirmation with its affected scope.
    RequestDestructiveConfirmation(DestructiveConfirmation),
    /// Confirms exactly the action currently awaiting confirmation.
    ConfirmDestructive {
        /// Must exactly match the currently confirmed destructive action.
        action_id: String,
    },
    /// Dismisses the outstanding destructive confirmation.
    CancelDestructiveConfirmation,
}

/// A typed shell effect for composition to dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobsEffect {
    /// Composition should request safe cancellation of this active job.
    CancelRequested {
        /// Identifies the active job for composition to cancel.
        job_id: JobId,
    },
    /// Composition should retry this recoverable job.
    RetryRequested {
        /// Identifies the recoverable job for composition to retry.
        job_id: JobId,
    },
    /// Composition should perform the explicitly confirmed destructive action.
    DestructiveConfirmed {
        /// Identifies the explicitly confirmed destructive operation.
        action_id: String,
    },
}

/// Local concurrent-job and destructive-confirmation state.
#[derive(Clone, Debug, Default)]
pub struct JobsController {
    jobs: BTreeMap<JobId, JobPresentation>,
    destructive_confirmation: Option<DestructiveConfirmation>,
}

impl JobsController {
    /// Records a named job and its latest immutable lifecycle state.
    pub fn observe_job(
        &mut self,
        descriptor: JobDescriptor,
        state: &JobState,
        progress: Option<JobProgress>,
    ) {
        let job_id = descriptor.job_id();
        self.jobs.insert(
            job_id,
            JobPresentation {
                descriptor,
                state: JobPresentationState::from_job_state(state),
                progress,
            },
        );
    }

    /// Applies a local interaction and returns a dispatchable effect only when it is valid.
    #[must_use]
    pub fn apply(&mut self, action: JobsAction) -> Option<JobsEffect> {
        match action {
            JobsAction::RequestCancel { job_id } => self
                .jobs
                .get(&job_id)
                .is_some_and(|job| job.state.can_cancel())
                .then_some(JobsEffect::CancelRequested { job_id }),
            JobsAction::Retry { job_id } => self
                .jobs
                .get(&job_id)
                .is_some_and(|job| job.state.can_retry())
                .then_some(JobsEffect::RetryRequested { job_id }),
            JobsAction::Dismiss { job_id } => self
                .jobs
                .get(&job_id)
                .is_some_and(|job| job.state.can_dismiss())
                .then(|| self.jobs.remove(&job_id))
                .flatten()
                .map(|_| JobsEffect::RetryRequested { job_id })
                .filter(|_| false),
            JobsAction::RequestDestructiveConfirmation(confirmation) => {
                self.destructive_confirmation = Some(confirmation);
                None
            }
            JobsAction::ConfirmDestructive { action_id } => self
                .destructive_confirmation
                .as_ref()
                .is_some_and(|confirmation| confirmation.action_id == action_id)
                .then(|| {
                    self.destructive_confirmation = None;
                    JobsEffect::DestructiveConfirmed { action_id }
                }),
            JobsAction::CancelDestructiveConfirmation => {
                self.destructive_confirmation = None;
                None
            }
        }
    }

    /// Produces the current complete component model without querying external adapters.
    #[must_use]
    pub fn view_model(&self) -> JobsViewModel {
        JobsViewModel {
            jobs: self.jobs.values().cloned().collect(),
            destructive_confirmation: self.destructive_confirmation.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use openmanic_application::{ApplicationError, ApplicationPort, PortFailureReason};

    use super::*;

    fn descriptor(job_id: u64) -> JobDescriptor {
        JobDescriptor::new(
            JobId::new(job_id),
            format!("Export {job_id}"),
            "1 Jan 2026 through 31 Jan 2026".to_owned(),
        )
    }

    #[test]
    fn named_concurrent_jobs_keep_their_states_progress_and_scope() {
        let mut controller = JobsController::default();
        controller.observe_job(
            descriptor(8),
            &JobState::Running,
            Some(JobProgress::new(4, Some(10), Some("Writing CSV".to_owned()))),
        );
        controller.observe_job(descriptor(3), &JobState::Cancelling, None);

        let view = controller.view_model();
        assert_eq!(view.jobs().len(), 2);
        assert_eq!(view.jobs()[0].job_id(), JobId::new(3));
        assert_eq!(view.jobs()[1].name(), "Export 8");
        assert_eq!(view.jobs()[1].scope(), "1 Jan 2026 through 31 Jan 2026");
        assert_eq!(view.jobs()[1].progress().map(JobProgress::completed), Some(4));
        assert!(matches!(view.jobs()[0].state(), JobPresentationState::Cancelling));
    }

    #[test]
    fn retry_and_dismiss_are_limited_to_recoverable_terminal_states() {
        let mut controller = JobsController::default();
        controller.observe_job(descriptor(5), &JobState::Running, None);
        assert_eq!(controller.apply(JobsAction::Retry { job_id: JobId::new(5) }), None);
        assert_eq!(controller.apply(JobsAction::Dismiss { job_id: JobId::new(5) }), None);

        controller.observe_job(
            descriptor(5),
            &JobState::Failed {
                error: ApplicationError::port_failure(
                    ApplicationPort::Command,
                    PortFailureReason::Unavailable,
                ),
            },
            None,
        );
        assert_eq!(
            controller.apply(JobsAction::Retry { job_id: JobId::new(5) }),
            Some(JobsEffect::RetryRequested { job_id: JobId::new(5) })
        );
        assert_eq!(controller.apply(JobsAction::Dismiss { job_id: JobId::new(5) }), None);
        assert!(controller.view_model().jobs().is_empty());
    }

    #[test]
    fn destructive_actions_require_matching_explicit_confirmation() {
        let mut controller = JobsController::default();
        let confirmation = DestructiveConfirmation::new(
            "restore-backup".to_owned(),
            "Restore backup?".to_owned(),
            "Current local data will be replaced.".to_owned(),
            "Restore".to_owned(),
        );
        assert_eq!(
            controller.apply(JobsAction::RequestDestructiveConfirmation(confirmation.clone())),
            None
        );
        assert_eq!(controller.view_model().destructive_confirmation(), Some(&confirmation));
        assert_eq!(
            controller.apply(JobsAction::ConfirmDestructive {
                action_id: "another-action".to_owned(),
            }),
            None
        );
        assert!(controller.view_model().destructive_confirmation().is_some());
        assert_eq!(
            controller.apply(JobsAction::ConfirmDestructive {
                action_id: "restore-backup".to_owned(),
            }),
            Some(JobsEffect::DestructiveConfirmed {
                action_id: "restore-backup".to_owned(),
            })
        );
        assert!(controller.view_model().destructive_confirmation().is_none());
    }
}
