//! Versioned application events and mutation reconciliation outcomes.

use core::fmt;

use openmanic_domain::UtcMicros;

use crate::{ApplicationError, CommandId, DataRevision, JobId, SchemaRevision, TrackingCheckpoint};

/// An event emitted by an application service with correlation metadata.
///
/// `E` is a typed event payload. Critical outcomes, including mutation results
/// and final job states, are represented as payload values rather than dropped
/// replaceable progress updates.
#[derive(Debug, Eq, PartialEq)]
pub struct EventEnvelope<E> {
    schema_revision: SchemaRevision,
    sequence: u64,
    causation_command_id: Option<CommandId>,
    committed_data_revision: Option<DataRevision>,
    occurred_at_utc: UtcMicros,
    payload: E,
}

impl<E> EventEnvelope<E> {
    /// Creates a versioned event envelope.
    #[must_use]
    pub const fn new(
        schema_revision: SchemaRevision,
        sequence: u64,
        causation_command_id: Option<CommandId>,
        committed_data_revision: Option<DataRevision>,
        occurred_at_utc: UtcMicros,
        payload: E,
    ) -> Self {
        Self {
            schema_revision,
            sequence,
            causation_command_id,
            committed_data_revision,
            occurred_at_utc,
            payload,
        }
    }

    /// Returns the event-shape schema revision.
    #[must_use]
    pub const fn schema_revision(&self) -> SchemaRevision {
        self.schema_revision
    }

    /// Returns the source-local event sequence number.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the originating command identifier when the event has one.
    #[must_use]
    pub const fn causation_command_id(&self) -> Option<CommandId> {
        self.causation_command_id
    }

    /// Returns the store revision committed atomically with this event, when any.
    #[must_use]
    pub const fn committed_data_revision(&self) -> Option<DataRevision> {
        self.committed_data_revision
    }

    /// Returns the authoritative UTC event timestamp.
    #[must_use]
    pub const fn occurred_at_utc(&self) -> UtcMicros {
        self.occurred_at_utc
    }

    /// Borrows the typed event payload.
    #[must_use]
    pub const fn payload(&self) -> &E {
        &self.payload
    }

    /// Consumes the envelope and returns its typed payload.
    #[must_use]
    pub fn into_payload(self) -> E {
        self.payload
    }
}

/// The authoritative result of one mutation command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationOutcome {
    /// The mutation committed successfully at the stated data revision.
    Confirmed(MutationConfirmation),
    /// The mutation was not committed and the UI must reconcile authoritative state.
    Rejected(MutationRejection),
}

/// Confirms a mutation command and the store revision it committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MutationConfirmation {
    command_id: CommandId,
    committed_data_revision: DataRevision,
}

impl MutationConfirmation {
    /// Creates authoritative confirmation for a committed command.
    #[must_use]
    pub const fn new(command_id: CommandId, committed_data_revision: DataRevision) -> Self {
        Self {
            command_id,
            committed_data_revision,
        }
    }

    /// Returns the confirmed command identifier.
    #[must_use]
    pub const fn command_id(self) -> CommandId {
        self.command_id
    }

    /// Returns the revision committed atomically with the mutation.
    #[must_use]
    pub const fn committed_data_revision(self) -> DataRevision {
        self.committed_data_revision
    }
}

/// Rejects a mutation command without claiming that its state was committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationRejection {
    command_id: CommandId,
    reason: MutationRejectionReason,
}

impl MutationRejection {
    /// Creates a typed rejection for an uncommitted command.
    #[must_use]
    pub const fn new(command_id: CommandId, reason: MutationRejectionReason) -> Self {
        Self { command_id, reason }
    }

    /// Returns the rejected command identifier.
    #[must_use]
    pub const fn command_id(&self) -> CommandId {
        self.command_id
    }

    /// Returns the typed reason the mutation could not be committed.
    #[must_use]
    pub const fn reason(&self) -> MutationRejectionReason {
        self.reason
    }
}

/// The stable category of an authoritative mutation rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationRejectionReason {
    /// The requested value failed domain or application validation.
    Validation,
    /// The command's expected entity revision no longer matched persisted state.
    RevisionConflict,
    /// Persistence could not commit the authoritative mutation.
    PersistenceFailure,
    /// The responsible application service was unavailable.
    ServiceUnavailable,
}

/// A typed tracking-service event carried by an [`AppEvent`] envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrackingEvent {
    /// An attempted tracking mutation reached an authoritative outcome.
    Mutation {
        /// The result correlated with the submitted command.
        outcome: MutationOutcome,
        /// The immutable checkpoint committed with a confirmed outcome.
        checkpoint: Option<TrackingCheckpoint>,
    },
    /// Evidence was deliberately ignored without fabricating an interval.
    EvidenceIgnored {
        /// Source sequence supplied by the platform adapter.
        sequence: u64,
        /// The reason the observation could not change canonical tracking state.
        reason: TrackingEvidenceIgnoredReason,
    },
    /// A valid command did not require a persistence mutation.
    NoAuthoritativeChange,
}

/// Explains why normalized evidence did not change tracking state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingEvidenceIgnoredReason {
    /// The adapter sequence repeated or arrived after a newer sequence.
    DuplicateOrReordered,
    /// The evidence time predates the current canonical open interval.
    TimestampBeforeCurrentState,
    /// A higher-precedence cause was already observed at the same instant.
    LowerPrecedenceAtSameInstant,
}

impl fmt::Display for MutationRejectionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::Validation => "validation failed",
            Self::RevisionConflict => "the entity revision changed",
            Self::PersistenceFailure => "persistence could not commit the mutation",
            Self::ServiceUnavailable => "the application service is unavailable",
        };
        formatter.write_str(description)
    }
}

/// A finite lifecycle state for a background job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobState {
    /// The job has been accepted but has not begun work.
    Queued,
    /// The job is currently executing.
    Running,
    /// Cancellation was requested and is awaiting a safe boundary.
    Cancelling,
    /// The job completed successfully.
    Succeeded,
    /// The job completed because cancellation was honored.
    Cancelled,
    /// The job ended with a typed application failure.
    Failed {
        /// The stable application error reported for the failed job.
        error: ApplicationError,
    },
    /// The job was interrupted before it reached a final outcome.
    Interrupted,
}

/// Reports the state of one background job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobEvent {
    job_id: JobId,
    state: JobState,
}

impl JobEvent {
    /// Creates an event for a job lifecycle state.
    #[must_use]
    pub fn new(job_id: JobId, state: JobState) -> Self {
        Self { job_id, state }
    }

    /// Returns the stable background-job identifier.
    #[must_use]
    pub const fn job_id(&self) -> JobId {
        self.job_id
    }

    /// Returns the reported lifecycle state.
    #[must_use]
    pub const fn state(&self) -> &JobState {
        &self.state
    }
}

/// Events whose variants are frozen at the application boundary for this phase.
///
/// Future application services may add explicitly reviewed variants, but callers
/// must preserve exhaustive handling of the currently declared critical outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AppEvent {
    /// A command's authoritative mutation result.
    Mutation(MutationOutcome),
    /// A background job lifecycle update.
    Job(JobEvent),
    /// A platform-neutral tracking outcome or evidence decision.
    Tracking(TrackingEvent),
}
