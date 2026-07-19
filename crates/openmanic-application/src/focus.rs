//! Focus-session commands, durable service coordination, and immutable snapshots.

use openmanic_domain::{
    CategoryId, FocusSession, FocusSessionError, FocusSessionId, FocusSessionState, UtcMicros,
};

use crate::{
    CommandEnvelope, DataRevision, EntityRevision, MutationConfirmation, MutationOutcome,
    MutationRejection, MutationRejectionReason,
};

/// The two locally rendered focus timer kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusKind {
    /// A productive focus interval.
    Focus,
    /// A short restorative break.
    ShortBreak,
}

/// A durable focus-session draft or lifecycle snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusSnapshot {
    session_id: FocusSessionId,
    kind: FocusKind,
    label: Option<String>,
    session: FocusSession,
    entity_revision: EntityRevision,
}

impl FocusSnapshot {
    /// Builds a new ready draft with a stable ID supplied by the caller.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError`] for an invalid duration and rejects blank labels.
    pub fn try_new(
        session_id: FocusSessionId,
        kind: FocusKind,
        label: Option<String>,
        intended_duration_us: i64,
        category_id: Option<CategoryId>,
        entity_revision: EntityRevision,
    ) -> Result<Self, FocusSessionError> {
        if label
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(FocusSessionError::InvalidRestoredState);
        }
        Ok(Self {
            session_id,
            kind,
            label,
            session: FocusSession::try_new(intended_duration_us, category_id)?,
            entity_revision,
        })
    }

    /// Rebuilds a durable snapshot after its SQLite row was decoded.
    ///
    /// # Errors
    ///
    /// Returns [`FocusSessionError`] when the stored lifecycle values or label
    /// do not satisfy the application and domain invariants.
    pub fn try_restore(
        session_id: FocusSessionId,
        kind: FocusKind,
        label: Option<String>,
        intended_duration_us: i64,
        category_id: Option<CategoryId>,
        state: FocusSessionState,
        entity_revision: EntityRevision,
    ) -> Result<Self, FocusSessionError> {
        if label
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(FocusSessionError::InvalidRestoredState);
        }
        Ok(Self {
            session_id,
            kind,
            label,
            session: FocusSession::try_restore(intended_duration_us, category_id, state)?,
            entity_revision,
        })
    }

    /// Returns the stable session identity.
    #[must_use]
    pub const fn session_id(&self) -> FocusSessionId {
        self.session_id
    }
    /// Returns the display kind.
    #[must_use]
    pub const fn kind(&self) -> FocusKind {
        self.kind
    }
    /// Returns the optional user-provided label.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
    /// Returns the authoritative state machine.
    #[must_use]
    pub const fn session(&self) -> &FocusSession {
        &self.session
    }
    /// Returns the optimistic-concurrency revision.
    #[must_use]
    pub const fn entity_revision(&self) -> EntityRevision {
        self.entity_revision
    }

    /// Applies a persisted revision after an atomic replacement.
    pub(crate) fn set_entity_revision(&mut self, revision: EntityRevision) {
        self.entity_revision = revision;
    }
}

/// Focus mutations submitted through the ordered focus singleton.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FocusCommand {
    /// Persists a ready or planned draft without starting it.
    CreateDraft(FocusSnapshot),
    /// Adds a user-selected planned interval to a ready draft.
    Plan {
        /// Stable identity of the ready draft being planned.
        session_id: FocusSessionId,
        /// User-selected UTC planning boundary.
        planned_start: UtcMicros,
        /// User-selected UTC planning boundary after `planned_start`.
        planned_end: UtcMicros,
    },
    /// Explicitly starts a ready or planned session.
    Start {
        /// Stable identity of the session to start.
        session_id: FocusSessionId,
        /// Authoritative wall-clock start timestamp.
        started_at: UtcMicros,
    },
    /// Freezes a running timer's remaining duration.
    Pause {
        /// Stable identity of the running session to pause.
        session_id: FocusSessionId,
        /// Authoritative wall-clock pause timestamp.
        paused_at: UtcMicros,
    },
    /// Restarts a paused timer from its preserved remaining duration.
    Resume {
        /// Stable identity of the paused session to resume.
        session_id: FocusSessionId,
        /// Authoritative wall-clock resume timestamp.
        resumed_at: UtcMicros,
    },
    /// Completes an active or paused timer before its deadline.
    Complete {
        /// Stable identity of the active or paused session to complete.
        session_id: FocusSessionId,
        /// Authoritative completion timestamp.
        completed_at: UtcMicros,
    },
    /// Cancels an active or paused timer.
    Cancel {
        /// Stable identity of the active or paused session to cancel.
        session_id: FocusSessionId,
        /// Authoritative cancellation timestamp.
        cancelled_at: UtcMicros,
    },
}

/// Durable operations required by the focus service.
pub trait FocusPersistence {
    /// Loads one focus snapshot by stable identity.
    ///
    /// # Errors
    ///
    /// Returns [`FocusPersistenceError`] when the durable boundary cannot read the snapshot.
    fn load_focus(
        &mut self,
        session_id: FocusSessionId,
    ) -> Result<Option<FocusSnapshot>, FocusPersistenceError>;
    /// Loads the single running or paused snapshot, if one exists.
    ///
    /// # Errors
    ///
    /// Returns [`FocusPersistenceError`] when the durable boundary cannot read the singleton.
    fn load_active_focus(&mut self) -> Result<Option<FocusSnapshot>, FocusPersistenceError>;
    /// Inserts a new ready or planned draft and returns its committed data revision.
    ///
    /// # Errors
    ///
    /// Returns [`FocusPersistenceError`] if the draft cannot be persisted atomically.
    fn create_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<DataRevision, FocusPersistenceError>;
    /// Replaces one snapshot if its expected entity revision still matches.
    ///
    /// # Errors
    ///
    /// Returns [`FocusPersistenceError`] if the snapshot is absent, stale, or cannot commit.
    fn replace_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<(DataRevision, EntityRevision), FocusPersistenceError>;
}

/// Stable persistence failures exposed by the focus boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusPersistenceError {
    /// The requested stable session identity is absent.
    Missing,
    /// A different running or paused session occupies the singleton slot.
    ActiveSessionExists,
    /// The durable entity revision no longer matches the submitted snapshot.
    RevisionConflict,
    /// The persistence adapter could not complete the requested operation.
    Failed,
}

/// Delivers an optional local completion notification after persistence succeeds.
pub trait FocusNotificationPort {
    /// Attempts a best-effort completion notification without changing durable state.
    ///
    /// # Errors
    ///
    /// Returns [`FocusNotificationError`] when local notification delivery is unavailable or fails.
    fn notify_completed(&mut self, snapshot: &FocusSnapshot) -> Result<(), FocusNotificationError>;
}

/// A notification failure that remains visible without retracting a committed completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusNotificationError {
    /// The local notification capability is currently unavailable.
    Unavailable,
    /// The notification adapter failed after a durable mutation committed.
    Failed,
}

/// Correlated focus command result and the latest immutable snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusMutation {
    outcome: MutationOutcome,
    snapshot: Option<FocusSnapshot>,
    notification_error: Option<FocusNotificationError>,
}

impl FocusMutation {
    fn confirmed(
        command: &CommandEnvelope<FocusCommand>,
        revision: DataRevision,
        snapshot: FocusSnapshot,
        notification_error: Option<FocusNotificationError>,
    ) -> Self {
        Self {
            outcome: MutationOutcome::Confirmed(MutationConfirmation::new(
                command.command_id(),
                revision,
            )),
            snapshot: Some(snapshot),
            notification_error,
        }
    }
    fn rejected(command: &CommandEnvelope<FocusCommand>, reason: MutationRejectionReason) -> Self {
        Self {
            outcome: MutationOutcome::Rejected(MutationRejection::new(
                command.command_id(),
                reason,
            )),
            snapshot: None,
            notification_error: None,
        }
    }
    /// Returns the authoritative mutation result.
    #[must_use]
    pub const fn outcome(&self) -> &MutationOutcome {
        &self.outcome
    }
    /// Returns the updated snapshot only after persistence confirmed it.
    #[must_use]
    pub fn snapshot(&self) -> Option<&FocusSnapshot> {
        self.snapshot.as_ref()
    }
    /// Returns an independent completion-notification failure, if any.
    #[must_use]
    pub const fn notification_error(&self) -> Option<FocusNotificationError> {
        self.notification_error
    }
}

/// Coordinates the one durable focus singleton without deriving time from repaint cadence.
pub struct FocusService<P, N> {
    persistence: P,
    notifications: N,
}

impl<P, N> FocusService<P, N>
where
    P: FocusPersistence,
    N: FocusNotificationPort,
{
    /// Creates a focus service over the exclusive persistence and notification ports.
    #[must_use]
    pub const fn new(persistence: P, notifications: N) -> Self {
        Self {
            persistence,
            notifications,
        }
    }

    /// Handles one ordered command and returns a correlated authoritative outcome.
    #[must_use]
    pub fn handle(&mut self, command: &CommandEnvelope<FocusCommand>) -> FocusMutation {
        match command.payload() {
            FocusCommand::CreateDraft(snapshot) => {
                if !matches!(
                    snapshot.session().state(),
                    FocusSessionState::Ready | FocusSessionState::Planned { .. }
                ) {
                    return FocusMutation::rejected(command, MutationRejectionReason::Validation);
                }
                match self.persistence.create_focus(snapshot) {
                    Ok(revision) => {
                        FocusMutation::confirmed(command, revision, snapshot.clone(), None)
                    }
                    Err(error) => FocusMutation::rejected(command, rejection(error)),
                }
            }
            FocusCommand::Plan {
                session_id,
                planned_start,
                planned_end,
            } => self.transition(command, *session_id, |session| {
                session.plan(*planned_start, *planned_end)
            }),
            FocusCommand::Start {
                session_id,
                started_at,
            } => match self.persistence.load_active_focus() {
                Ok(Some(active)) if active.session_id() != *session_id => {
                    FocusMutation::rejected(command, MutationRejectionReason::Validation)
                }
                Ok(_) => {
                    self.transition(command, *session_id, |session| session.start(*started_at))
                }
                Err(error) => FocusMutation::rejected(command, rejection(error)),
            },
            FocusCommand::Pause {
                session_id,
                paused_at,
            } => self.transition(command, *session_id, |session| session.pause(*paused_at)),
            FocusCommand::Resume {
                session_id,
                resumed_at,
            } => self.transition(command, *session_id, |session| session.resume(*resumed_at)),
            FocusCommand::Complete {
                session_id,
                completed_at,
            } => self.transition(command, *session_id, |session| {
                session.complete(*completed_at)
            }),
            FocusCommand::Cancel {
                session_id,
                cancelled_at,
            } => self.transition(command, *session_id, |session| {
                session.cancel(*cancelled_at)
            }),
        }
    }

    /// Reconciles a persisted active timer after restart and persists only a real deadline transition.
    ///
    /// # Errors
    ///
    /// Returns [`FocusPersistenceError`] if the active snapshot cannot be loaded or its
    /// reconciliation cannot commit.
    pub fn reconcile_after_restart(
        &mut self,
        restarted_at: UtcMicros,
    ) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        let Some(mut snapshot) = self.persistence.load_active_focus()? else {
            return Ok(None);
        };
        let before = snapshot.session.state();
        snapshot.session.reconcile_after_restart(restarted_at);
        if snapshot.session.state() == before {
            return Ok(Some(snapshot));
        }
        let (_, revision) = self.persistence.replace_focus(&snapshot)?;
        snapshot.set_entity_revision(revision);
        let _ = self.notifications.notify_completed(&snapshot);
        Ok(Some(snapshot))
    }

    fn transition(
        &mut self,
        command: &CommandEnvelope<FocusCommand>,
        session_id: FocusSessionId,
        operation: impl FnOnce(&mut FocusSession) -> Result<(), FocusSessionError>,
    ) -> FocusMutation {
        let mut snapshot = match self.persistence.load_focus(session_id) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                return FocusMutation::rejected(command, MutationRejectionReason::Validation);
            }
            Err(error) => return FocusMutation::rejected(command, rejection(error)),
        };
        if operation(&mut snapshot.session).is_err() {
            return FocusMutation::rejected(command, MutationRejectionReason::Validation);
        }
        let completed = matches!(
            snapshot.session.state(),
            FocusSessionState::Completed { .. }
        );
        match self.persistence.replace_focus(&snapshot) {
            Ok((revision, entity_revision)) => {
                snapshot.set_entity_revision(entity_revision);
                let notification_error = if completed {
                    self.notifications.notify_completed(&snapshot).err()
                } else {
                    None
                };
                FocusMutation::confirmed(command, revision, snapshot, notification_error)
            }
            Err(error) => FocusMutation::rejected(command, rejection(error)),
        }
    }
}

fn rejection(error: FocusPersistenceError) -> MutationRejectionReason {
    match error {
        FocusPersistenceError::RevisionConflict => MutationRejectionReason::RevisionConflict,
        FocusPersistenceError::Missing | FocusPersistenceError::ActiveSessionExists => {
            MutationRejectionReason::Validation
        }
        FocusPersistenceError::Failed => MutationRejectionReason::PersistenceFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandId, OrderingKey, SchemaRevision};
    use openmanic_domain::FocusSessionId;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MemoryPersistence {
        values: BTreeMap<[u8; 16], FocusSnapshot>,
        revision: u64,
    }
    impl FocusPersistence for MemoryPersistence {
        fn load_focus(
            &mut self,
            id: FocusSessionId,
        ) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
            Ok(self.values.get(&id.as_bytes()).cloned())
        }
        fn load_active_focus(&mut self) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
            Ok(self
                .values
                .values()
                .find(|value| value.session().is_active_or_paused())
                .cloned())
        }
        fn create_focus(
            &mut self,
            value: &FocusSnapshot,
        ) -> Result<DataRevision, FocusPersistenceError> {
            self.revision += 1;
            self.values
                .insert(value.session_id().as_bytes(), value.clone());
            Ok(DataRevision::new(self.revision))
        }
        fn replace_focus(
            &mut self,
            value: &FocusSnapshot,
        ) -> Result<(DataRevision, EntityRevision), FocusPersistenceError> {
            let Some(previous) = self.values.get(&value.session_id().as_bytes()) else {
                return Err(FocusPersistenceError::Missing);
            };
            if previous.entity_revision() != value.entity_revision() {
                return Err(FocusPersistenceError::RevisionConflict);
            }
            self.revision += 1;
            let mut stored = value.clone();
            stored.set_entity_revision(EntityRevision::new(value.entity_revision().get() + 1));
            let entity = stored.entity_revision();
            self.values.insert(value.session_id().as_bytes(), stored);
            Ok((DataRevision::new(self.revision), entity))
        }
    }
    #[derive(Default)]
    struct Notifications {
        fail: bool,
        count: usize,
    }
    impl FocusNotificationPort for Notifications {
        fn notify_completed(&mut self, _: &FocusSnapshot) -> Result<(), FocusNotificationError> {
            self.count += 1;
            if self.fail {
                Err(FocusNotificationError::Unavailable)
            } else {
                Ok(())
            }
        }
    }
    fn command(id: u64, payload: FocusCommand) -> CommandEnvelope<FocusCommand> {
        CommandEnvelope::new(
            SchemaRevision::new(1),
            CommandId::new(id),
            OrderingKey::new(1),
            None,
            UtcMicros::new(0),
            payload,
        )
    }
    fn draft(byte: u8) -> FocusSnapshot {
        FocusSnapshot::try_new(
            FocusSessionId::from_bytes([byte; 16]),
            FocusKind::Focus,
            None,
            50,
            None,
            EntityRevision::new(0),
        )
        .expect("fixture draft is valid")
    }

    #[test]
    fn starts_pauses_and_completes_from_authoritative_timestamps() {
        let mut service = FocusService::new(MemoryPersistence::default(), Notifications::default());
        assert!(matches!(
            service
                .handle(&command(1, FocusCommand::CreateDraft(draft(1))))
                .outcome(),
            MutationOutcome::Confirmed(_)
        ));
        let id = draft(1).session_id();
        assert!(matches!(
            service
                .handle(&command(
                    2,
                    FocusCommand::Start {
                        session_id: id,
                        started_at: UtcMicros::new(100)
                    }
                ))
                .outcome(),
            MutationOutcome::Confirmed(_)
        ));
        assert!(matches!(
            service
                .handle(&command(
                    3,
                    FocusCommand::Pause {
                        session_id: id,
                        paused_at: UtcMicros::new(110)
                    }
                ))
                .snapshot()
                .map(|value| value.session().state()),
            Some(FocusSessionState::Paused {
                remaining_us: 40,
                ..
            })
        ));
        assert!(matches!(
            service
                .handle(&command(
                    4,
                    FocusCommand::Complete {
                        session_id: id,
                        completed_at: UtcMicros::new(120)
                    }
                ))
                .outcome(),
            MutationOutcome::Confirmed(_)
        ));
    }

    #[test]
    fn restart_deadline_completion_is_persisted_without_a_repaint_tick() {
        let mut service = FocusService::new(MemoryPersistence::default(), Notifications::default());
        let id = draft(2).session_id();
        let _ = service.handle(&command(1, FocusCommand::CreateDraft(draft(2))));
        let _ = service.handle(&command(
            2,
            FocusCommand::Start {
                session_id: id,
                started_at: UtcMicros::new(100),
            },
        ));
        let restored = service
            .reconcile_after_restart(UtcMicros::new(200))
            .expect("fixture persistence succeeds")
            .expect("active session exists");
        assert!(
            matches!(restored.session().state(), FocusSessionState::Completed { completed_at, .. } if completed_at == UtcMicros::new(150))
        );
    }

    #[test]
    fn notification_failure_does_not_retract_a_committed_completion() {
        let mut service = FocusService::new(
            MemoryPersistence::default(),
            Notifications {
                fail: true,
                count: 0,
            },
        );
        let id = draft(3).session_id();
        let _ = service.handle(&command(1, FocusCommand::CreateDraft(draft(3))));
        let _ = service.handle(&command(
            2,
            FocusCommand::Start {
                session_id: id,
                started_at: UtcMicros::new(100),
            },
        ));
        let outcome = service.handle(&command(
            3,
            FocusCommand::Complete {
                session_id: id,
                completed_at: UtcMicros::new(110),
            },
        ));
        assert!(matches!(outcome.outcome(), MutationOutcome::Confirmed(_)));
        assert_eq!(
            outcome.notification_error(),
            Some(FocusNotificationError::Unavailable)
        );
    }
}
