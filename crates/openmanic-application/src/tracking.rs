//! Platform-neutral activity tracking reduction and transactional persistence intents.
//!
//! The reducer receives normalized platform evidence and explicit user commands.
//! It owns canonical open-interval state but delegates every durable mutation to a
//! fakeable atomic port. It deliberately has no platform, SQLite, runtime, or UI
//! dependency.

use core::fmt;

use openmanic_domain::{
    ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, ApplicationId,
    HalfOpenInterval, PowerTransitionEvidence, TrackerRunId, UtcMicros,
};

use crate::{
    AppEvent, CommandEnvelope, CommandId, DataRevision, EventEnvelope, MutationConfirmation,
    MutationOutcome, MutationRejection, MutationRejectionReason, SchemaRevision, TrackingCommand,
    TrackingEvent, TrackingEvidence, TrackingEvidenceIgnoredReason, TrackingPersistenceSubmit,
};

/// The normal maximum interval between durable open-activity confirmations.
pub const TRACKING_CHECKPOINT_INTERVAL_US: i64 = 5_000_000;

/// A durable representation of the currently open canonical activity interval.
///
/// A checkpoint's `last_confirmed_utc` limits recovery: an unexpected exit must
/// not extend an attributed activity interval beyond that instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackingCheckpoint {
    tracker_run_id: TrackerRunId,
    open_start_utc: UtcMicros,
    last_confirmed_utc: UtcMicros,
    state: ActivityState,
    evidence: ActivityEvidence,
    application_id: Option<ApplicationId>,
    platform_sequence: u64,
}

impl TrackingCheckpoint {
    /// Creates a valid non-powered-off open activity checkpoint.
    ///
    /// # Errors
    ///
    /// Returns [`TrackingCheckpointError`] if time, activity state, or power
    /// evidence would make the open interval invalid.
    pub fn try_new(
        tracker_run_id: TrackerRunId,
        open_start_utc: UtcMicros,
        last_confirmed_utc: UtcMicros,
        state: ActivityState,
        evidence: ActivityEvidence,
        application_id: Option<ApplicationId>,
        platform_sequence: u64,
    ) -> Result<Self, TrackingCheckpointError> {
        if last_confirmed_utc < open_start_utc {
            return Err(TrackingCheckpointError::ConfirmationBeforeOpenStart {
                open_start_utc,
                last_confirmed_utc,
            });
        }
        if state == ActivityState::PoweredOff {
            return Err(TrackingCheckpointError::PoweredOffMustBeClosed);
        }
        if evidence.power_transition().is_some() {
            return Err(TrackingCheckpointError::PowerTransitionMustBeClosed);
        }
        if state == ActivityState::Active && application_id.is_none() {
            return Err(TrackingCheckpointError::ApplicationRequiredForActive);
        }
        if state != ActivityState::Active && application_id.is_some() {
            return Err(TrackingCheckpointError::ApplicationForbiddenForState { state });
        }
        Ok(Self {
            tracker_run_id,
            open_start_utc,
            last_confirmed_utc,
            state,
            evidence,
            application_id,
            platform_sequence,
        })
    }

    /// Returns the tracker run that owns this checkpoint.
    #[must_use]
    pub const fn tracker_run_id(self) -> TrackerRunId {
        self.tracker_run_id
    }

    /// Returns the inclusive start of the still-open canonical interval.
    #[must_use]
    pub const fn open_start_utc(self) -> UtcMicros {
        self.open_start_utc
    }

    /// Returns the latest instant durably confirmed for recovery.
    #[must_use]
    pub const fn last_confirmed_utc(self) -> UtcMicros {
        self.last_confirmed_utc
    }

    /// Returns the current canonical activity state.
    #[must_use]
    pub const fn state(self) -> ActivityState {
        self.state
    }

    /// Returns the explicit evidence cause for the current state.
    #[must_use]
    pub const fn cause(self) -> ActivityCause {
        self.evidence.cause()
    }

    /// Returns the source evidence that qualifies the current state.
    #[must_use]
    pub const fn evidence(self) -> ActivityEvidence {
        self.evidence
    }

    /// Returns the resolved application only for active tracking.
    #[must_use]
    pub const fn application_id(self) -> Option<ApplicationId> {
        self.application_id
    }

    /// Returns the greatest platform sequence durably represented by this checkpoint.
    #[must_use]
    pub const fn platform_sequence(self) -> u64 {
        self.platform_sequence
    }

    fn with_confirmation(self, confirmed_at: UtcMicros, platform_sequence: u64) -> Self {
        Self {
            last_confirmed_utc: confirmed_at,
            platform_sequence,
            ..self
        }
    }

    fn into_closed_interval(self, end: UtcMicros) -> Option<ActivityInterval> {
        let Ok(range) = HalfOpenInterval::try_new(self.open_start_utc, end) else {
            return None;
        };
        ActivityInterval::try_new(
            self.tracker_run_id,
            range,
            self.state,
            self.evidence,
            self.application_id,
        )
        .ok()
    }
}

/// Explains why a proposed open-activity checkpoint is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingCheckpointError {
    /// The durable confirmation predates the open interval's start.
    ConfirmationBeforeOpenStart {
        /// Inclusive open interval start.
        open_start_utc: UtcMicros,
        /// Requested durable confirmation instant.
        last_confirmed_utc: UtcMicros,
    },
    /// An active interval requires a resolved application identity.
    ApplicationRequiredForActive,
    /// A non-active state must not retain application attribution.
    ApplicationForbiddenForState {
        /// State that rejects the application identity.
        state: ActivityState,
    },
    /// Powered Off is a finite, qualifying interval rather than an open state.
    PoweredOffMustBeClosed,
    /// Power transition evidence can only qualify a finite Powered Off interval.
    PowerTransitionMustBeClosed,
}

impl fmt::Display for TrackingCheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfirmationBeforeOpenStart {
                open_start_utc,
                last_confirmed_utc,
            } => write!(
                formatter,
                "checkpoint confirmation {} predates open start {}",
                last_confirmed_utc.get(),
                open_start_utc.get()
            ),
            Self::ApplicationRequiredForActive => {
                formatter.write_str("active checkpoint requires an application")
            }
            Self::ApplicationForbiddenForState { state } => {
                write!(
                    formatter,
                    "{state:?} checkpoint must not retain an application"
                )
            }
            Self::PoweredOffMustBeClosed => {
                formatter.write_str("powered-off activity must be persisted as a closed interval")
            }
            Self::PowerTransitionMustBeClosed => formatter
                .write_str("power-transition evidence must be persisted as a closed interval"),
        }
    }
}

impl std::error::Error for TrackingCheckpointError {}

/// A complete atomic persistence request for a tracking transition or checkpoint.
///
/// `closed_intervals` is chronologically ordered and cannot overlap each other or
/// the represented open checkpoint. The storage implementation commits every
/// member together with its data revision, or commits none of them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackingPersistenceIntent {
    closed_intervals: Vec<ActivityInterval>,
    checkpoint: TrackingCheckpoint,
}

impl TrackingPersistenceIntent {
    /// Creates an ordered, non-overlapping tracking persistence request.
    ///
    /// # Errors
    ///
    /// Returns [`TrackingPersistenceIntentError`] when intervals overlap or
    /// extend into the open checkpoint.
    pub fn try_new(
        closed_intervals: Vec<ActivityInterval>,
        checkpoint: TrackingCheckpoint,
    ) -> Result<Self, TrackingPersistenceIntentError> {
        let mut previous_end = None;
        for interval in &closed_intervals {
            if interval.tracker_run_id() != checkpoint.tracker_run_id() {
                return Err(TrackingPersistenceIntentError::TrackerRunMismatch);
            }
            if let Some(end) = previous_end
                && interval.range().start() < end
            {
                return Err(TrackingPersistenceIntentError::ClosedIntervalsOverlap);
            }
            if interval.range().end() > checkpoint.open_start_utc() {
                return Err(TrackingPersistenceIntentError::ClosedIntervalOverlapsOpenState);
            }
            previous_end = Some(interval.range().end());
        }
        Ok(Self {
            closed_intervals,
            checkpoint,
        })
    }

    /// Returns closed canonical intervals that must commit with the checkpoint.
    #[must_use]
    pub fn closed_intervals(&self) -> &[ActivityInterval] {
        &self.closed_intervals
    }

    /// Returns the open state committed together with closed transition history.
    #[must_use]
    pub const fn checkpoint(&self) -> TrackingCheckpoint {
        self.checkpoint
    }
}

/// Explains why a tracking persistence request would violate interval ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingPersistenceIntentError {
    /// One closed interval belongs to a different tracker run.
    TrackerRunMismatch,
    /// Two closed intervals overlap instead of using adjacent half-open boundaries.
    ClosedIntervalsOverlap,
    /// A closed interval reaches past the represented open interval's start.
    ClosedIntervalOverlapsOpenState,
}

impl fmt::Display for TrackingPersistenceIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::TrackerRunMismatch => "tracking intent mixes tracker runs",
            Self::ClosedIntervalsOverlap => "tracking intent closed intervals overlap",
            Self::ClosedIntervalOverlapsOpenState => {
                "tracking intent closed interval overlaps the open checkpoint"
            }
        };
        formatter.write_str(description)
    }
}

impl std::error::Error for TrackingPersistenceIntentError {}

/// Reduces normalized evidence into canonical intervals and transactional intents.
///
/// The service advances its authoritative state only after the persistence port
/// confirms an atomic commit. A retained writer request remains available through
/// [`Self::pending_intent`] and may be retried with [`Self::retry_pending`].
#[derive(Debug)]
pub struct TrackingService<P> {
    persistence: P,
    tracker_run_id: TrackerRunId,
    checkpoint: Option<TrackingCheckpoint>,
    user_paused: bool,
    last_platform_sequence: Option<u64>,
    emitted_event_sequence: u64,
    pending: Option<PendingTrackingMutation>,
}

#[derive(Clone, Debug)]
struct PendingTrackingMutation {
    intent: TrackingPersistenceIntent,
    next_checkpoint: TrackingCheckpoint,
    next_user_paused: bool,
    next_platform_sequence: Option<u64>,
    schema_revision: SchemaRevision,
    command_id: CommandId,
    occurred_at_utc: UtcMicros,
}

#[derive(Debug)]
struct TrackingTransition {
    schema_revision: SchemaRevision,
    command_id: CommandId,
    occurred_at_utc: UtcMicros,
    next_start: UtcMicros,
    target: StateDescriptor,
    next_platform_sequence: Option<u64>,
    next_user_paused: bool,
    closed_intervals: Vec<ActivityInterval>,
    close_current_at: Option<UtcMicros>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionCloseError {
    BeforeCurrentState,
    InvalidBoundary,
}

impl<P> TrackingService<P>
where
    P: crate::TrackingPersistencePort,
{
    /// Creates a tracking service for one already-created tracker run.
    #[must_use]
    pub fn new(tracker_run_id: TrackerRunId, persistence: P) -> Self {
        Self {
            persistence,
            tracker_run_id,
            checkpoint: None,
            user_paused: false,
            last_platform_sequence: None,
            emitted_event_sequence: 0,
            pending: None,
        }
    }

    /// Returns the current committed checkpoint, if trusted tracking has started.
    #[must_use]
    pub const fn checkpoint(&self) -> Option<TrackingCheckpoint> {
        self.checkpoint
    }

    /// Returns whether an explicit user pause currently dominates platform evidence.
    #[must_use]
    pub const fn is_user_paused(&self) -> bool {
        self.user_paused
    }

    /// Returns a retained atomic mutation that must be retried or surfaced at shutdown.
    #[must_use]
    pub fn pending_intent(&self) -> Option<&TrackingPersistenceIntent> {
        self.pending.as_ref().map(|pending| &pending.intent)
    }

    /// Returns the source-local sequence most recently accepted by this process.
    #[must_use]
    pub const fn last_platform_sequence(&self) -> Option<u64> {
        self.last_platform_sequence
    }

    /// Handles one explicit command or normalized platform evidence item.
    ///
    /// A full persistence lane is never treated as a successful mutation. The
    /// complete intent remains pending until [`Self::retry_pending`] confirms it.
    #[must_use]
    pub fn handle(&mut self, command: CommandEnvelope<TrackingCommand>) -> EventEnvelope<AppEvent> {
        let schema_revision = command.schema_revision();
        let command_id = command.command_id();
        let submitted_at_utc = command.submitted_at_utc();
        let payload = command.into_payload();

        if self.pending.is_some() {
            return self.rejected_event(
                schema_revision,
                command_id,
                submitted_at_utc,
                MutationRejectionReason::ServiceUnavailable,
            );
        }

        match payload {
            TrackingCommand::Pause => {
                self.reduce_pause(schema_revision, command_id, submitted_at_utc)
            }
            TrackingCommand::Resume => {
                self.reduce_resume(schema_revision, command_id, submitted_at_utc)
            }
            TrackingCommand::Checkpoint => {
                self.reduce_checkpoint(schema_revision, command_id, submitted_at_utc)
            }
            TrackingCommand::Evidence(evidence) => {
                self.reduce_evidence(schema_revision, command_id, evidence)
            }
        }
    }

    /// Immediately replaces trusted attribution for the active application with an excluded
    /// state after its privacy policy changes. This is local reconciliation, so it deliberately
    /// preserves the last accepted platform sequence for the next real foreground observation.
    #[must_use]
    pub fn reconcile_active_application_excluded(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
        application_id: ApplicationId,
    ) -> EventEnvelope<AppEvent> {
        if self.pending.is_some()
            || self.user_paused
            || self
                .checkpoint
                .is_none_or(|checkpoint| checkpoint.application_id() != Some(application_id))
        {
            return self.no_change_event(schema_revision, command_id, at);
        }
        let Some(target) = state_descriptor(
            ActivityState::Excluded,
            ActivityCause::ApplicationExcluded,
            None,
        ) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                at,
                MutationRejectionReason::Validation,
            );
        };
        self.persist_transition(TrackingTransition {
            schema_revision,
            command_id,
            occurred_at_utc: at,
            next_start: at,
            target,
            next_platform_sequence: self.last_platform_sequence,
            next_user_paused: false,
            closed_intervals: Vec::new(),
            close_current_at: None,
        })
    }

    /// Retries the one retained atomic mutation, if writer backpressure left one pending.
    #[must_use]
    pub fn retry_pending(&mut self) -> Option<EventEnvelope<AppEvent>> {
        let pending = self.pending.take()?;
        match self.persistence.try_persist(pending.intent.clone()) {
            TrackingPersistenceSubmit::Committed(revision) => {
                self.apply_committed(&pending);
                Some(self.confirmed_event(&pending, revision))
            }
            TrackingPersistenceSubmit::Retained { intent, .. } => {
                self.pending = Some(PendingTrackingMutation { intent, ..pending });
                Some(self.rejected_event(
                    pending.schema_revision,
                    pending.command_id,
                    pending.occurred_at_utc,
                    MutationRejectionReason::ServiceUnavailable,
                ))
            }
            TrackingPersistenceSubmit::Failed(_error) => Some(self.rejected_event(
                pending.schema_revision,
                pending.command_id,
                pending.occurred_at_utc,
                MutationRejectionReason::PersistenceFailure,
            )),
        }
    }

    /// Consumes the service and returns its adapter-owned persistence implementation.
    #[must_use]
    pub fn into_persistence(self) -> P {
        self.persistence
    }

    fn reduce_pause(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
    ) -> EventEnvelope<AppEvent> {
        if self.user_paused {
            return self.no_change_event(schema_revision, command_id, at);
        }
        let Some(target) =
            state_descriptor(ActivityState::PausedByUser, ActivityCause::UserPause, None)
        else {
            return self.rejected_event(
                schema_revision,
                command_id,
                at,
                MutationRejectionReason::Validation,
            );
        };
        self.persist_transition(TrackingTransition {
            schema_revision,
            command_id,
            occurred_at_utc: at,
            next_start: at,
            target,
            next_platform_sequence: self.last_platform_sequence,
            next_user_paused: true,
            closed_intervals: Vec::new(),
            close_current_at: None,
        })
    }

    fn reduce_resume(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
    ) -> EventEnvelope<AppEvent> {
        if !self.user_paused {
            return self.no_change_event(schema_revision, command_id, at);
        }
        let Some(target) = state_descriptor(
            ActivityState::UnknownMissing,
            ActivityCause::AdapterStarting,
            None,
        ) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                at,
                MutationRejectionReason::Validation,
            );
        };
        self.persist_transition(TrackingTransition {
            schema_revision,
            command_id,
            occurred_at_utc: at,
            next_start: at,
            target,
            next_platform_sequence: self.last_platform_sequence,
            next_user_paused: false,
            closed_intervals: Vec::new(),
            close_current_at: None,
        })
    }

    fn reduce_checkpoint(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
    ) -> EventEnvelope<AppEvent> {
        let Some(current) = self.checkpoint else {
            return self.no_change_event(schema_revision, command_id, at);
        };
        if at < current.last_confirmed_utc() || !checkpoint_due(current.last_confirmed_utc(), at) {
            return self.no_change_event(schema_revision, command_id, at);
        }
        let updated = current.with_confirmation(at, self.last_platform_sequence.unwrap_or(0));
        self.persist_checkpoint(
            schema_revision,
            command_id,
            at,
            updated,
            self.user_paused,
            self.last_platform_sequence,
        )
    }

    fn reduce_evidence(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        evidence: TrackingEvidence,
    ) -> EventEnvelope<AppEvent> {
        let sequence = evidence.sequence();
        let at = evidence.observed_at_utc();
        if self
            .last_platform_sequence
            .is_some_and(|last_sequence| sequence <= last_sequence)
        {
            return self.ignored_event(
                schema_revision,
                command_id,
                at,
                sequence,
                TrackingEvidenceIgnoredReason::DuplicateOrReordered,
            );
        }

        if self.user_paused {
            self.last_platform_sequence = Some(sequence);
            return self.no_change_event(schema_revision, command_id, at);
        }

        if let TrackingEvidence::ConfirmedPowerTransition { boundaries, .. } = evidence {
            return self.reduce_confirmed_power(schema_revision, command_id, sequence, boundaries);
        }

        let Some(target) = descriptor_from_evidence(evidence) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                at,
                MutationRejectionReason::Validation,
            );
        };
        self.reduce_descriptor_evidence(schema_revision, command_id, sequence, at, target)
    }

    fn reduce_descriptor_evidence(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        sequence: u64,
        at: UtcMicros,
        target: StateDescriptor,
    ) -> EventEnvelope<AppEvent> {
        let Some(current) = self.checkpoint else {
            return self.persist_transition(TrackingTransition {
                schema_revision,
                command_id,
                occurred_at_utc: at,
                next_start: at,
                target,
                next_platform_sequence: Some(sequence),
                next_user_paused: false,
                closed_intervals: Vec::new(),
                close_current_at: None,
            });
        };
        if at < current.open_start_utc() {
            self.last_platform_sequence = Some(sequence);
            return self.ignored_event(
                schema_revision,
                command_id,
                at,
                sequence,
                TrackingEvidenceIgnoredReason::TimestampBeforeCurrentState,
            );
        }
        if at == current.open_start_utc()
            && target.precedence() < precedence_for_checkpoint(current)
        {
            self.last_platform_sequence = Some(sequence);
            return self.ignored_event(
                schema_revision,
                command_id,
                at,
                sequence,
                TrackingEvidenceIgnoredReason::LowerPrecedenceAtSameInstant,
            );
        }
        if target.matches(current) {
            self.last_platform_sequence = Some(sequence);
            if at < current.last_confirmed_utc()
                || !checkpoint_due(current.last_confirmed_utc(), at)
            {
                return self.no_change_event(schema_revision, command_id, at);
            }
            let updated = current.with_confirmation(at, sequence);
            return self.persist_checkpoint(
                schema_revision,
                command_id,
                at,
                updated,
                false,
                Some(sequence),
            );
        }
        self.persist_transition(TrackingTransition {
            schema_revision,
            command_id,
            occurred_at_utc: at,
            next_start: at,
            target,
            next_platform_sequence: Some(sequence),
            next_user_paused: false,
            closed_intervals: Vec::new(),
            close_current_at: None,
        })
    }

    fn reduce_confirmed_power(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        sequence: u64,
        boundaries: PowerTransitionEvidence,
    ) -> EventEnvelope<AppEvent> {
        let shutdown = boundaries.shutdown();
        let startup = boundaries.startup();
        if self
            .checkpoint
            .is_some_and(|checkpoint| shutdown < checkpoint.open_start_utc())
        {
            self.last_platform_sequence = Some(sequence);
            return self.ignored_event(
                schema_revision,
                command_id,
                startup,
                sequence,
                TrackingEvidenceIgnoredReason::TimestampBeforeCurrentState,
            );
        }

        let Some(unknown_after_startup) = state_descriptor(
            ActivityState::UnknownMissing,
            ActivityCause::AdapterStarting,
            None,
        ) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                startup,
                MutationRejectionReason::Validation,
            );
        };
        let power_evidence = ActivityEvidence::confirmed_shutdown(boundaries);
        let Ok(power_range) = HalfOpenInterval::try_new(shutdown, startup) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                startup,
                MutationRejectionReason::Validation,
            );
        };
        let Ok(powered_off) = ActivityInterval::try_new(
            self.tracker_run_id,
            power_range,
            ActivityState::PoweredOff,
            power_evidence,
            None,
        ) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                startup,
                MutationRejectionReason::Validation,
            );
        };

        self.persist_transition(TrackingTransition {
            schema_revision,
            command_id,
            occurred_at_utc: startup,
            next_start: startup,
            target: unknown_after_startup,
            next_platform_sequence: Some(sequence),
            next_user_paused: false,
            closed_intervals: vec![powered_off],
            close_current_at: Some(shutdown),
        })
    }

    fn persist_transition(
        &mut self,
        mut transition: TrackingTransition,
    ) -> EventEnvelope<AppEvent> {
        let TrackingTransition {
            schema_revision,
            command_id,
            occurred_at_utc,
            next_start,
            target,
            next_platform_sequence,
            next_user_paused,
            close_current_at,
            ..
        } = transition;
        if self
            .checkpoint
            .is_some_and(|checkpoint| next_start < checkpoint.open_start_utc())
        {
            return self.no_change_event(schema_revision, command_id, occurred_at_utc);
        }
        match self.append_closed_current_interval(&mut transition, close_current_at) {
            Ok(()) => {}
            Err(TransitionCloseError::BeforeCurrentState) => {
                return self.no_change_event(schema_revision, command_id, occurred_at_utc);
            }
            Err(TransitionCloseError::InvalidBoundary) => {
                return self.rejected_event(
                    schema_revision,
                    command_id,
                    occurred_at_utc,
                    MutationRejectionReason::Validation,
                );
            }
        }
        let sequence = next_platform_sequence.unwrap_or_else(|| {
            self.last_platform_sequence
                .or_else(|| self.checkpoint.map(TrackingCheckpoint::platform_sequence))
                .unwrap_or(0)
        });
        let Ok(next_checkpoint) = TrackingCheckpoint::try_new(
            self.tracker_run_id,
            next_start,
            next_start,
            target.state,
            target.evidence,
            target.application_id,
            sequence,
        ) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                occurred_at_utc,
                MutationRejectionReason::Validation,
            );
        };
        let Ok(intent) =
            TrackingPersistenceIntent::try_new(transition.closed_intervals, next_checkpoint)
        else {
            return self.rejected_event(
                schema_revision,
                command_id,
                occurred_at_utc,
                MutationRejectionReason::Validation,
            );
        };
        let pending = PendingTrackingMutation {
            intent,
            next_checkpoint,
            next_user_paused,
            next_platform_sequence,
            schema_revision,
            command_id,
            occurred_at_utc,
        };
        self.persist_intent(&pending)
    }

    fn persist_checkpoint(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        occurred_at_utc: UtcMicros,
        next_checkpoint: TrackingCheckpoint,
        next_user_paused: bool,
        next_platform_sequence: Option<u64>,
    ) -> EventEnvelope<AppEvent> {
        let Ok(intent) = TrackingPersistenceIntent::try_new(Vec::new(), next_checkpoint) else {
            return self.rejected_event(
                schema_revision,
                command_id,
                occurred_at_utc,
                MutationRejectionReason::Validation,
            );
        };
        let pending = PendingTrackingMutation {
            intent,
            next_checkpoint,
            next_user_paused,
            next_platform_sequence,
            schema_revision,
            command_id,
            occurred_at_utc,
        };
        self.persist_intent(&pending)
    }

    fn append_closed_current_interval(
        &self,
        transition: &mut TrackingTransition,
        close_current_at: Option<UtcMicros>,
    ) -> Result<(), TransitionCloseError> {
        let Some(current) = self.checkpoint else {
            return Ok(());
        };
        let boundary = close_current_at.unwrap_or(transition.next_start);
        if boundary > transition.next_start {
            return Err(TransitionCloseError::InvalidBoundary);
        }
        if boundary < current.open_start_utc() {
            return Err(TransitionCloseError::BeforeCurrentState);
        }
        if boundary == current.open_start_utc() {
            return Ok(());
        }
        let Some(closed) = current.into_closed_interval(boundary) else {
            return Err(TransitionCloseError::InvalidBoundary);
        };
        transition.closed_intervals.insert(0, closed);
        Ok(())
    }

    fn persist_intent(&mut self, pending: &PendingTrackingMutation) -> EventEnvelope<AppEvent> {
        match self.persistence.try_persist(pending.intent.clone()) {
            TrackingPersistenceSubmit::Committed(revision) => {
                self.apply_committed(pending);
                self.confirmed_event(pending, revision)
            }
            TrackingPersistenceSubmit::Retained { intent, .. } => {
                let schema_revision = pending.schema_revision;
                let command_id = pending.command_id;
                let occurred_at_utc = pending.occurred_at_utc;
                let mut retained = pending.clone();
                retained.intent = intent;
                self.pending = Some(retained);
                self.rejected_event(
                    schema_revision,
                    command_id,
                    occurred_at_utc,
                    MutationRejectionReason::ServiceUnavailable,
                )
            }
            TrackingPersistenceSubmit::Failed(_error) => self.rejected_event(
                pending.schema_revision,
                pending.command_id,
                pending.occurred_at_utc,
                MutationRejectionReason::PersistenceFailure,
            ),
        }
    }

    fn apply_committed(&mut self, pending: &PendingTrackingMutation) {
        self.checkpoint = Some(pending.next_checkpoint);
        self.user_paused = pending.next_user_paused;
        if let Some(sequence) = pending.next_platform_sequence {
            self.last_platform_sequence = Some(sequence);
        }
    }

    fn confirmed_event(
        &mut self,
        pending: &PendingTrackingMutation,
        revision: DataRevision,
    ) -> EventEnvelope<AppEvent> {
        self.event(
            pending.schema_revision,
            pending.command_id,
            pending.occurred_at_utc,
            Some(revision),
            AppEvent::Tracking(TrackingEvent::Mutation {
                outcome: MutationOutcome::Confirmed(MutationConfirmation::new(
                    pending.command_id,
                    revision,
                )),
                checkpoint: Some(pending.next_checkpoint),
            }),
        )
    }

    fn rejected_event(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
        reason: MutationRejectionReason,
    ) -> EventEnvelope<AppEvent> {
        self.event(
            schema_revision,
            command_id,
            at,
            None,
            AppEvent::Tracking(TrackingEvent::Mutation {
                outcome: MutationOutcome::Rejected(MutationRejection::new(command_id, reason)),
                checkpoint: None,
            }),
        )
    }

    fn ignored_event(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
        evidence_sequence: u64,
        reason: TrackingEvidenceIgnoredReason,
    ) -> EventEnvelope<AppEvent> {
        self.event(
            schema_revision,
            command_id,
            at,
            None,
            AppEvent::Tracking(TrackingEvent::EvidenceIgnored {
                sequence: evidence_sequence,
                reason,
            }),
        )
    }

    fn no_change_event(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
    ) -> EventEnvelope<AppEvent> {
        self.event(
            schema_revision,
            command_id,
            at,
            None,
            AppEvent::Tracking(TrackingEvent::NoAuthoritativeChange),
        )
    }

    fn event(
        &mut self,
        schema_revision: SchemaRevision,
        command_id: CommandId,
        at: UtcMicros,
        revision: Option<DataRevision>,
        payload: AppEvent,
    ) -> EventEnvelope<AppEvent> {
        self.emitted_event_sequence = self.emitted_event_sequence.saturating_add(1);
        EventEnvelope::new(
            schema_revision,
            self.emitted_event_sequence,
            Some(command_id),
            revision,
            at,
            payload,
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct StateDescriptor {
    state: ActivityState,
    evidence: ActivityEvidence,
    application_id: Option<ApplicationId>,
}

impl StateDescriptor {
    fn matches(self, checkpoint: TrackingCheckpoint) -> bool {
        self.state == checkpoint.state()
            && self.evidence == checkpoint.evidence()
            && self.application_id == checkpoint.application_id()
    }

    const fn precedence(self) -> u8 {
        precedence(self.state, self.evidence.cause())
    }
}

fn descriptor_from_evidence(evidence: TrackingEvidence) -> Option<StateDescriptor> {
    match evidence {
        TrackingEvidence::Foreground { application_id, .. } => state_descriptor(
            ActivityState::Active,
            ActivityCause::ForegroundApplication,
            Some(application_id),
        ),
        TrackingEvidence::ExcludedForeground { .. } => state_descriptor(
            ActivityState::Excluded,
            ActivityCause::ApplicationExcluded,
            None,
        ),
        TrackingEvidence::IdleThresholdCrossed { .. } => {
            state_descriptor(ActivityState::Idle, ActivityCause::IdleThreshold, None)
        }
        TrackingEvidence::SessionLocked { .. } => state_descriptor(
            ActivityState::Unavailable,
            ActivityCause::SessionLocked,
            None,
        ),
        TrackingEvidence::SessionDisconnected { .. } => state_descriptor(
            ActivityState::Unavailable,
            ActivityCause::SessionDisconnected,
            None,
        ),
        TrackingEvidence::SystemSuspended { .. } => state_descriptor(
            ActivityState::Unavailable,
            ActivityCause::SystemSuspended,
            None,
        ),
        TrackingEvidence::AdapterStarting { .. } => state_descriptor(
            ActivityState::UnknownMissing,
            ActivityCause::AdapterStarting,
            None,
        ),
        TrackingEvidence::AdapterPermissionLost { .. } => state_descriptor(
            ActivityState::Unavailable,
            ActivityCause::AdapterPermissionLost,
            None,
        ),
        TrackingEvidence::AdapterFailure { .. } => state_descriptor(
            ActivityState::Unavailable,
            ActivityCause::AdapterFailure,
            None,
        ),
        TrackingEvidence::EvidenceQueueOverflow { .. } => state_descriptor(
            ActivityState::UnknownMissing,
            ActivityCause::EvidenceQueueOverflow,
            None,
        ),
        TrackingEvidence::ClockDiscontinuity { .. } => state_descriptor(
            ActivityState::UnknownMissing,
            ActivityCause::ClockDiscontinuity,
            None,
        ),
        TrackingEvidence::ConfirmedPowerTransition { .. } => None,
    }
}

fn state_descriptor(
    state: ActivityState,
    cause: ActivityCause,
    application_id: Option<ApplicationId>,
) -> Option<StateDescriptor> {
    let evidence = ActivityEvidence::try_from_cause(cause).ok()?;
    Some(StateDescriptor {
        state,
        evidence,
        application_id,
    })
}

const fn precedence_for_checkpoint(checkpoint: TrackingCheckpoint) -> u8 {
    precedence(checkpoint.state(), checkpoint.cause())
}

const fn precedence(state: ActivityState, cause: ActivityCause) -> u8 {
    match (state, cause) {
        (ActivityState::PausedByUser, ActivityCause::UserPause) => 70,
        (ActivityState::PoweredOff, ActivityCause::ConfirmedShutdown) => 60,
        (
            ActivityState::Unavailable,
            ActivityCause::SessionLocked
            | ActivityCause::SessionDisconnected
            | ActivityCause::SystemSuspended,
        ) => 50,
        (
            ActivityState::Unavailable,
            ActivityCause::AdapterPermissionLost | ActivityCause::AdapterFailure,
        )
        | (
            ActivityState::UnknownMissing,
            ActivityCause::AdapterStarting
            | ActivityCause::EvidenceQueueOverflow
            | ActivityCause::ClockDiscontinuity,
        ) => 40,
        (ActivityState::Excluded, ActivityCause::ApplicationExcluded) => 30,
        (ActivityState::Idle, ActivityCause::IdleThreshold) => 20,
        (ActivityState::Active, ActivityCause::ForegroundApplication) => 10,
        _ => 0,
    }
}

fn checkpoint_due(last_confirmed: UtcMicros, at: UtcMicros) -> bool {
    at.checked_difference(last_confirmed)
        .is_ok_and(|elapsed| elapsed >= TRACKING_CHECKPOINT_INTERVAL_US)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use openmanic_domain::{ApplicationId, PowerTransitionEvidence, TrackerRunId, UtcMicros};

    use super::{TRACKING_CHECKPOINT_INTERVAL_US, TrackingPersistenceIntent, TrackingService};
    use crate::{
        AppEvent, ApplicationError, ApplicationPort, CommandEnvelope, CommandId, DataRevision,
        MutationOutcome, MutationRejectionReason, OrderingKey, PortFailureReason, SchemaRevision,
        TrackingCommand, TrackingEvent, TrackingEvidence, TrackingEvidenceIgnoredReason,
        TrackingPersistencePort, TrackingPersistenceRetentionReason, TrackingPersistenceSubmit,
    };

    #[test]
    fn duplicate_and_reordered_evidence_never_reopens_or_overlaps_activity() {
        let mut service = service([SubmitPlan::Commit(1)]);
        let first = service.handle(evidence_command(
            1,
            1,
            TrackingEvidence::Foreground {
                sequence: 7,
                observed_at_utc: time(10),
                application_id: app(1),
            },
        ));
        assert_confirmed(&first, 1);

        for (command, sequence) in [(2, 7), (3, 6)] {
            let event = service.handle(evidence_command(
                command,
                2,
                TrackingEvidence::Foreground {
                    sequence,
                    observed_at_utc: time(11),
                    application_id: app(2),
                },
            ));
            assert_eq!(
                event.payload(),
                &AppEvent::Tracking(TrackingEvent::EvidenceIgnored {
                    sequence,
                    reason: TrackingEvidenceIgnoredReason::DuplicateOrReordered,
                })
            );
        }
        assert_eq!(
            service.checkpoint().expect("first commit").application_id(),
            Some(app(1))
        );
    }

    #[test]
    fn rapid_a_to_b_to_a_is_committed_as_adjacent_non_overlapping_intervals() {
        let mut service = service([
            SubmitPlan::Commit(1),
            SubmitPlan::Commit(2),
            SubmitPlan::Commit(3),
        ]);
        let _ = service.handle(evidence_command(
            1,
            0,
            TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: time(0),
                application_id: app(1),
            },
        ));
        let _ = service.handle(evidence_command(
            2,
            10,
            TrackingEvidence::Foreground {
                sequence: 2,
                observed_at_utc: time(10),
                application_id: app(2),
            },
        ));
        let _ = service.handle(evidence_command(
            3,
            20,
            TrackingEvidence::Foreground {
                sequence: 3,
                observed_at_utc: time(20),
                application_id: app(1),
            },
        ));

        let persistence = service.into_persistence();
        assert_eq!(persistence.intents.len(), 3);
        let first_closed = persistence.intents[1].closed_intervals()[0];
        let second_closed = persistence.intents[2].closed_intervals()[0];
        assert_eq!(first_closed.application_id(), Some(app(1)));
        assert_eq!(second_closed.application_id(), Some(app(2)));
        assert!(
            first_closed
                .range()
                .is_immediately_before(second_closed.range())
        );
        assert!(!first_closed.range().overlaps(second_closed.range()));
    }

    #[test]
    fn excluding_the_active_application_closes_attribution_without_consuming_future_sequence() {
        let mut service = service([
            SubmitPlan::Commit(1),
            SubmitPlan::Commit(2),
            SubmitPlan::Commit(3),
        ]);
        let _ = service.handle(evidence_command(
            1,
            0,
            TrackingEvidence::Foreground {
                sequence: 7,
                observed_at_utc: time(0),
                application_id: app(1),
            },
        ));
        let reconciliation = service.reconcile_active_application_excluded(
            SchemaRevision::new(1),
            CommandId::new(2),
            time(10),
            app(1),
        );
        assert_confirmed(&reconciliation, 2);
        assert_eq!(service.last_platform_sequence(), Some(7));
        assert_eq!(
            service
                .checkpoint()
                .expect("reconciliation committed")
                .state(),
            openmanic_domain::ActivityState::Excluded
        );
        assert_eq!(
            service
                .checkpoint()
                .expect("reconciliation committed")
                .application_id(),
            None
        );
        assert_confirmed(
            &service.handle(evidence_command(
                3,
                11,
                TrackingEvidence::Foreground {
                    sequence: 8,
                    observed_at_utc: time(11),
                    application_id: app(2),
                },
            )),
            3,
        );
    }

    #[test]
    fn pause_dominates_same_instant_foreground_and_resume_requires_fresh_evidence() {
        let mut service = service([
            SubmitPlan::Commit(1),
            SubmitPlan::Commit(2),
            SubmitPlan::Commit(3),
        ]);
        let _ = service.handle(evidence_command(
            1,
            10,
            TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: time(10),
                application_id: app(1),
            },
        ));
        assert_confirmed(&service.handle(command(2, 10, TrackingCommand::Pause)), 2);
        let ignored = service.handle(evidence_command(
            3,
            10,
            TrackingEvidence::Foreground {
                sequence: 2,
                observed_at_utc: time(10),
                application_id: app(2),
            },
        ));
        assert_eq!(
            ignored.payload(),
            &AppEvent::Tracking(TrackingEvent::NoAuthoritativeChange)
        );
        assert!(service.is_user_paused());
        assert_eq!(
            service.checkpoint().expect("pause committed").state(),
            openmanic_domain::ActivityState::PausedByUser
        );
        assert_confirmed(&service.handle(command(4, 12, TrackingCommand::Resume)), 3);
        assert!(!service.is_user_paused());
        assert_eq!(
            service.checkpoint().expect("resume committed").state(),
            openmanic_domain::ActivityState::UnknownMissing
        );
    }

    #[test]
    fn permission_loss_and_queue_overflow_preserve_explicit_unavailable_or_unknown_causes() {
        let mut service = service([
            SubmitPlan::Commit(1),
            SubmitPlan::Commit(2),
            SubmitPlan::Commit(3),
        ]);
        let _ = service.handle(evidence_command(
            1,
            0,
            TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: time(0),
                application_id: app(1),
            },
        ));
        let _ = service.handle(evidence_command(
            2,
            10,
            TrackingEvidence::AdapterPermissionLost {
                sequence: 2,
                observed_at_utc: time(10),
            },
        ));
        assert_eq!(
            service.checkpoint().expect("permission checkpoint").cause(),
            openmanic_domain::ActivityCause::AdapterPermissionLost
        );
        let _ = service.handle(evidence_command(
            3,
            20,
            TrackingEvidence::EvidenceQueueOverflow {
                sequence: 3,
                observed_at_utc: time(20),
            },
        ));
        let checkpoint = service.checkpoint().expect("overflow checkpoint");
        assert_eq!(
            checkpoint.state(),
            openmanic_domain::ActivityState::UnknownMissing
        );
        assert_eq!(
            checkpoint.cause(),
            openmanic_domain::ActivityCause::EvidenceQueueOverflow
        );
    }

    #[test]
    fn checkpoint_policy_uses_fake_time_and_only_writes_after_five_seconds() {
        let mut service = service([SubmitPlan::Commit(1), SubmitPlan::Commit(2)]);
        let _ = service.handle(evidence_command(
            1,
            0,
            TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: time(0),
                application_id: app(1),
            },
        ));
        let early = service.handle(command(
            2,
            TRACKING_CHECKPOINT_INTERVAL_US - 1,
            TrackingCommand::Checkpoint,
        ));
        assert_eq!(
            early.payload(),
            &AppEvent::Tracking(TrackingEvent::NoAuthoritativeChange)
        );
        assert_confirmed(
            &service.handle(command(
                3,
                TRACKING_CHECKPOINT_INTERVAL_US,
                TrackingCommand::Checkpoint,
            )),
            2,
        );
        let persistence = service.into_persistence();
        assert_eq!(persistence.intents.len(), 2);
        assert!(persistence.intents[1].closed_intervals().is_empty());
        assert_eq!(
            persistence.intents[1].checkpoint().last_confirmed_utc(),
            time(TRACKING_CHECKPOINT_INTERVAL_US)
        );
    }

    #[test]
    fn retained_atomic_intent_blocks_reordering_then_retries_with_original_correlation() {
        let mut service = service([SubmitPlan::Retain, SubmitPlan::Commit(9)]);
        let rejected = service.handle(evidence_command(
            41,
            0,
            TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: time(0),
                application_id: app(1),
            },
        ));
        assert_rejected(&rejected, 41, MutationRejectionReason::ServiceUnavailable);
        assert!(service.pending_intent().is_some());
        let blocked = service.handle(command(42, 1, TrackingCommand::Pause));
        assert_rejected(&blocked, 42, MutationRejectionReason::ServiceUnavailable);

        let retried = service.retry_pending().expect("retained intent retries");
        assert_confirmed(&retried, 9);
        assert_eq!(retried.causation_command_id(), Some(CommandId::new(41)));
        assert!(service.pending_intent().is_none());
    }

    #[test]
    fn only_qualifying_power_evidence_creates_powered_off_and_starts_unknown_after_startup() {
        let mut service = service([SubmitPlan::Commit(1), SubmitPlan::Commit(2)]);
        let _ = service.handle(evidence_command(
            1,
            0,
            TrackingEvidence::Foreground {
                sequence: 1,
                observed_at_utc: time(0),
                application_id: app(1),
            },
        ));
        let boundaries =
            PowerTransitionEvidence::try_new(time(20), time(30)).expect("positive power span");
        assert_confirmed(
            &service.handle(evidence_command(
                2,
                30,
                TrackingEvidence::ConfirmedPowerTransition {
                    sequence: 2,
                    boundaries,
                },
            )),
            2,
        );
        let persistence = service.into_persistence();
        let intent = &persistence.intents[1];
        assert_eq!(intent.closed_intervals().len(), 2);
        assert_eq!(
            intent.closed_intervals()[1].state(),
            openmanic_domain::ActivityState::PoweredOff
        );
        assert_eq!(
            intent.checkpoint().state(),
            openmanic_domain::ActivityState::UnknownMissing
        );
        assert_eq!(intent.checkpoint().open_start_utc(), time(30));
    }

    #[derive(Debug)]
    enum SubmitPlan {
        Commit(u64),
        Retain,
        Fail,
    }

    #[derive(Debug)]
    struct FakePersistence {
        plans: VecDeque<SubmitPlan>,
        intents: Vec<TrackingPersistenceIntent>,
    }

    impl FakePersistence {
        fn new(plans: impl IntoIterator<Item = SubmitPlan>) -> Self {
            Self {
                plans: plans.into_iter().collect(),
                intents: Vec::new(),
            }
        }
    }

    impl TrackingPersistencePort for FakePersistence {
        fn try_persist(&mut self, intent: TrackingPersistenceIntent) -> TrackingPersistenceSubmit {
            match self.plans.pop_front().unwrap_or(SubmitPlan::Fail) {
                SubmitPlan::Commit(revision) => {
                    self.intents.push(intent);
                    TrackingPersistenceSubmit::Committed(DataRevision::new(revision))
                }
                SubmitPlan::Retain => TrackingPersistenceSubmit::Retained {
                    intent,
                    reason: TrackingPersistenceRetentionReason::QueueFull,
                },
                SubmitPlan::Fail => {
                    TrackingPersistenceSubmit::Failed(ApplicationError::port_failure(
                        ApplicationPort::Command,
                        PortFailureReason::Failed,
                    ))
                }
            }
        }
    }

    fn service(plans: impl IntoIterator<Item = SubmitPlan>) -> TrackingService<FakePersistence> {
        TrackingService::new(
            TrackerRunId::from_bytes([7; 16]),
            FakePersistence::new(plans),
        )
    }

    fn command(
        command_id: u64,
        at: i64,
        payload: TrackingCommand,
    ) -> CommandEnvelope<TrackingCommand> {
        CommandEnvelope::new(
            SchemaRevision::new(1),
            CommandId::new(command_id),
            OrderingKey::new(1),
            None,
            time(at),
            payload,
        )
    }

    fn evidence_command(
        command_id: u64,
        at: i64,
        evidence: TrackingEvidence,
    ) -> CommandEnvelope<TrackingCommand> {
        command(command_id, at, TrackingCommand::Evidence(evidence))
    }

    fn app(byte: u8) -> ApplicationId {
        ApplicationId::from_bytes([byte; 16])
    }

    fn time(value: i64) -> UtcMicros {
        UtcMicros::new(value)
    }

    fn assert_confirmed(event: &crate::EventEnvelope<AppEvent>, revision: u64) {
        assert_eq!(
            event.committed_data_revision(),
            Some(DataRevision::new(revision))
        );
        assert!(matches!(
            event.payload(),
            AppEvent::Tracking(TrackingEvent::Mutation {
                outcome: MutationOutcome::Confirmed(_),
                checkpoint: Some(_),
            })
        ));
    }

    fn assert_rejected(
        event: &crate::EventEnvelope<AppEvent>,
        command_id: u64,
        reason: MutationRejectionReason,
    ) {
        assert_eq!(
            event.causation_command_id(),
            Some(CommandId::new(command_id))
        );
        assert_eq!(
            event.payload(),
            &AppEvent::Tracking(TrackingEvent::Mutation {
                outcome: MutationOutcome::Rejected(crate::MutationRejection::new(
                    CommandId::new(command_id),
                    reason,
                )),
                checkpoint: None,
            })
        );
    }
}
