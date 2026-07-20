//! Typed schedule commands and immutable snapshots for the ordered schedule service.

use openmanic_domain::{
    OneTimeScheduleId, ScheduleEditScope, ScheduleRule, ScheduleSeriesId, UtcMicros,
};

use crate::{
    CommandEnvelope, DataRevision, EntityRevision, MutationConfirmation, MutationOutcome,
    MutationRejection, MutationRejectionReason,
};

/// Stable identity of one personal schedule entity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleId {
    /// A standalone one-time schedule item.
    OneTime(OneTimeScheduleId),
    /// A recurring schedule series and its occurrence lineage.
    Series(ScheduleSeriesId),
}

/// Immutable authoritative schedule fact supplied to persistence or presentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleSnapshot {
    id: ScheduleId,
    rule: ScheduleRule,
    entity_revision: EntityRevision,
    created_at_utc: UtcMicros,
}

impl ScheduleSnapshot {
    /// Creates a snapshot whose identity form agrees with the supplied rule form.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleSnapshotError`] when a one-time ID carries a recurring rule or vice versa.
    pub fn try_new(
        id: ScheduleId,
        rule: ScheduleRule,
        entity_revision: EntityRevision,
        created_at_utc: UtcMicros,
    ) -> Result<Self, ScheduleSnapshotError> {
        if matches!(
            (id, rule.is_repeating()),
            (ScheduleId::OneTime(_), true) | (ScheduleId::Series(_), false)
        ) {
            return Err(ScheduleSnapshotError::IdentityDoesNotMatchRule);
        }
        Ok(Self {
            id,
            rule,
            entity_revision,
            created_at_utc,
        })
    }

    /// Returns the stable schedule identity.
    #[must_use]
    pub const fn id(&self) -> ScheduleId {
        self.id
    }
    /// Returns the validated schedule rule.
    #[must_use]
    pub const fn rule(&self) -> &ScheduleRule {
        &self.rule
    }
    /// Returns the optimistic entity revision.
    #[must_use]
    pub const fn entity_revision(&self) -> EntityRevision {
        self.entity_revision
    }
    /// Returns the authoritative creation timestamp.
    #[must_use]
    pub const fn created_at_utc(&self) -> UtcMicros {
        self.created_at_utc
    }
}

/// Invalid immutable schedule snapshot construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleSnapshotError {
    /// The stable identity form did not agree with the rule's one-time or recurring form.
    IdentityDoesNotMatchRule,
}

/// A schedule mutation accepted by the ordered schedule service.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleCommand {
    /// Creates one validated personal schedule without mutating tracked activity.
    Create(ScheduleSnapshot),
    /// Replaces one schedule after the caller applies its explicit edit scope to the rule.
    Replace(ScheduleSnapshot),
    /// Deletes one schedule after the caller confirms the explicit deletion scope.
    Delete(ScheduleId),
    /// Deletes a recurring occurrence at the specified explicit scope.
    DeleteOccurrence {
        /// The stable recurring series identity.
        series_id: ScheduleSeriesId,
        /// The selected occurrence's local civil anchor date.
        anchor_date: i32,
        /// The user-confirmed recurrence deletion scope.
        scope: ScheduleEditScope,
    },
}

/// Durable schedule operations required by the application service.
pub trait SchedulePersistence {
    /// Loads one authoritative schedule snapshot for a scope-specific mutation.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulePersistenceError`] when the snapshot cannot be read safely.
    fn load_schedule(
        &mut self,
        schedule_id: ScheduleId,
    ) -> Result<Option<ScheduleSnapshot>, SchedulePersistenceError>;

    /// Atomically creates a schedule and returns the committed store revision.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulePersistenceError`] if persistence rejects or cannot commit the snapshot.
    fn create_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
    ) -> Result<DataRevision, SchedulePersistenceError>;

    /// Atomically replaces an existing schedule after checking its optimistic entity revision.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulePersistenceError`] when the schedule changed, the replacement conflicts,
    /// or the transaction cannot commit.
    fn replace_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError>;

    /// Atomically deletes an existing schedule after checking its optimistic entity revision.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulePersistenceError`] when the schedule changed or the transaction cannot
    /// commit.
    fn delete_schedule(
        &mut self,
        schedule_id: ScheduleId,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError>;
}

/// Stable persistence failures exposed by the schedule boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulePersistenceError {
    /// A persisted schedule or resolved occurrence conflicts with an existing personal schedule.
    Conflict,
    /// The targeted schedule no longer exists or has a different entity revision.
    RevisionConflict,
    /// The persistence adapter failed without committing the requested mutation.
    Failed,
}

/// Correlated result from one schedule command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleMutation {
    outcome: MutationOutcome,
    snapshot: Option<ScheduleSnapshot>,
}

impl ScheduleMutation {
    fn confirmed(
        command: &CommandEnvelope<ScheduleCommand>,
        revision: DataRevision,
        snapshot: ScheduleSnapshot,
    ) -> Self {
        Self {
            outcome: MutationOutcome::Confirmed(MutationConfirmation::new(
                command.command_id(),
                revision,
            )),
            snapshot: Some(snapshot),
        }
    }
    fn rejected(
        command: &CommandEnvelope<ScheduleCommand>,
        reason: MutationRejectionReason,
    ) -> Self {
        Self {
            outcome: MutationOutcome::Rejected(MutationRejection::new(
                command.command_id(),
                reason,
            )),
            snapshot: None,
        }
    }
    fn confirmed_without_snapshot(
        command: &CommandEnvelope<ScheduleCommand>,
        revision: DataRevision,
    ) -> Self {
        Self {
            outcome: MutationOutcome::Confirmed(MutationConfirmation::new(
                command.command_id(),
                revision,
            )),
            snapshot: None,
        }
    }
    /// Returns the authoritative mutation result.
    #[must_use]
    pub const fn outcome(&self) -> &MutationOutcome {
        &self.outcome
    }
    /// Returns the created immutable schedule only after a successful commit.
    #[must_use]
    pub fn snapshot(&self) -> Option<&ScheduleSnapshot> {
        self.snapshot.as_ref()
    }
}

/// Applies ordered schedule commands through the sole persistence authority.
pub struct ScheduleService<P> {
    persistence: P,
}

impl<P> ScheduleService<P>
where
    P: SchedulePersistence,
{
    /// Creates a schedule service around its exclusive persistence port.
    #[must_use]
    pub const fn new(persistence: P) -> Self {
        Self { persistence }
    }

    /// Executes one schedule command without optimistic local persistence.
    #[must_use]
    pub fn handle(&mut self, command: &CommandEnvelope<ScheduleCommand>) -> ScheduleMutation {
        match command.payload() {
            ScheduleCommand::Create(snapshot) => match self.persistence.create_schedule(snapshot) {
                Ok(revision) => ScheduleMutation::confirmed(command, revision, snapshot.clone()),
                Err(SchedulePersistenceError::Conflict) => {
                    ScheduleMutation::rejected(command, MutationRejectionReason::Validation)
                }
                Err(SchedulePersistenceError::RevisionConflict) => {
                    ScheduleMutation::rejected(command, MutationRejectionReason::RevisionConflict)
                }
                Err(SchedulePersistenceError::Failed) => {
                    ScheduleMutation::rejected(command, MutationRejectionReason::PersistenceFailure)
                }
            },
            ScheduleCommand::Replace(snapshot) => {
                let Some(expected_revision) = command.expected_entity_revision() else {
                    return ScheduleMutation::rejected(
                        command,
                        MutationRejectionReason::RevisionConflict,
                    );
                };
                match self.persistence.replace_schedule(snapshot, expected_revision) {
                    Ok(revision) => ScheduleMutation::confirmed(command, revision, snapshot.clone()),
                    Err(SchedulePersistenceError::Conflict) => {
                        ScheduleMutation::rejected(command, MutationRejectionReason::Validation)
                    }
                    Err(SchedulePersistenceError::RevisionConflict) => {
                        ScheduleMutation::rejected(command, MutationRejectionReason::RevisionConflict)
                    }
                    Err(SchedulePersistenceError::Failed) => ScheduleMutation::rejected(
                        command,
                        MutationRejectionReason::PersistenceFailure,
                    ),
                }
            }
            ScheduleCommand::Delete(schedule_id) => {
                let Some(expected_revision) = command.expected_entity_revision() else {
                    return ScheduleMutation::rejected(
                        command,
                        MutationRejectionReason::RevisionConflict,
                    );
                };
                match self.persistence.delete_schedule(*schedule_id, expected_revision) {
                    Ok(revision) => ScheduleMutation::confirmed_without_snapshot(command, revision),
                    Err(SchedulePersistenceError::RevisionConflict) => {
                        ScheduleMutation::rejected(command, MutationRejectionReason::RevisionConflict)
                    }
                    Err(SchedulePersistenceError::Conflict) => {
                        ScheduleMutation::rejected(command, MutationRejectionReason::Validation)
                    }
                    Err(SchedulePersistenceError::Failed) => ScheduleMutation::rejected(
                        command,
                        MutationRejectionReason::PersistenceFailure,
                    ),
                }
            }
            ScheduleCommand::DeleteOccurrence {
                series_id,
                anchor_date,
                scope,
            } => self.delete_occurrence(command, *series_id, *anchor_date, *scope),
        }
    }

    fn delete_occurrence(
        &mut self,
        command: &CommandEnvelope<ScheduleCommand>,
        series_id: ScheduleSeriesId,
        anchor_date: i32,
        scope: ScheduleEditScope,
    ) -> ScheduleMutation {
        let Some(expected_revision) = command.expected_entity_revision() else {
            return ScheduleMutation::rejected(command, MutationRejectionReason::RevisionConflict);
        };
        let schedule_id = ScheduleId::Series(series_id);
        let snapshot = match self.persistence.load_schedule(schedule_id) {
            Ok(Some(snapshot)) if snapshot.entity_revision() == expected_revision => snapshot,
            Ok(Some(_)) => {
                return ScheduleMutation::rejected(
                    command,
                    MutationRejectionReason::RevisionConflict,
                );
            }
            Ok(None) => {
                return ScheduleMutation::rejected(
                    command,
                    MutationRejectionReason::RevisionConflict,
                );
            }
            Err(SchedulePersistenceError::Conflict | SchedulePersistenceError::RevisionConflict) => {
                return ScheduleMutation::rejected(
                    command,
                    MutationRejectionReason::RevisionConflict,
                );
            }
            Err(SchedulePersistenceError::Failed) => {
                return ScheduleMutation::rejected(
                    command,
                    MutationRejectionReason::PersistenceFailure,
                );
            }
        };
        if scope == ScheduleEditScope::EveryOccurrence {
            return self.delete_loaded_schedule(command, schedule_id, expected_revision);
        }
        let mut rule = snapshot.rule().clone();
        let delete_series = match scope {
            ScheduleEditScope::OnlyThisDate => rule.skip_only_this_date(anchor_date).map(|()| false),
            ScheduleEditScope::ThisAndFuture => rule.delete_this_and_future(anchor_date),
            ScheduleEditScope::EveryOccurrence => unreachable!("handled before rule mutation"),
        };
        let delete_series = match delete_series {
            Ok(delete_series) => delete_series,
            Err(_) => {
                return ScheduleMutation::rejected(command, MutationRejectionReason::Validation);
            }
        };
        if delete_series {
            return self.delete_loaded_schedule(command, schedule_id, expected_revision);
        }
        let replacement = match ScheduleSnapshot::try_new(
            schedule_id,
            rule,
            snapshot.entity_revision(),
            snapshot.created_at_utc(),
        ) {
            Ok(replacement) => replacement,
            Err(_) => return ScheduleMutation::rejected(command, MutationRejectionReason::Validation),
        };
        match self.persistence.replace_schedule(&replacement, expected_revision) {
            Ok(revision) => ScheduleMutation::confirmed(command, revision, replacement),
            Err(SchedulePersistenceError::Conflict) => {
                ScheduleMutation::rejected(command, MutationRejectionReason::Validation)
            }
            Err(SchedulePersistenceError::RevisionConflict) => {
                ScheduleMutation::rejected(command, MutationRejectionReason::RevisionConflict)
            }
            Err(SchedulePersistenceError::Failed) => {
                ScheduleMutation::rejected(command, MutationRejectionReason::PersistenceFailure)
            }
        }
    }

    fn delete_loaded_schedule(
        &mut self,
        command: &CommandEnvelope<ScheduleCommand>,
        schedule_id: ScheduleId,
        expected_revision: EntityRevision,
    ) -> ScheduleMutation {
        match self.persistence.delete_schedule(schedule_id, expected_revision) {
            Ok(revision) => ScheduleMutation::confirmed_without_snapshot(command, revision),
            Err(SchedulePersistenceError::RevisionConflict) => {
                ScheduleMutation::rejected(command, MutationRejectionReason::RevisionConflict)
            }
            Err(SchedulePersistenceError::Conflict) => {
                ScheduleMutation::rejected(command, MutationRejectionReason::Validation)
            }
            Err(SchedulePersistenceError::Failed) => {
                ScheduleMutation::rejected(command, MutationRejectionReason::PersistenceFailure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use openmanic_domain::{
        HalfOpenInterval, OneTimeScheduleId, ScheduleEditScope, ScheduleRule, ScheduleSeriesId,
        UtcMicros,
    };

    use super::{
        ScheduleCommand, ScheduleId, SchedulePersistence, SchedulePersistenceError,
        ScheduleService, ScheduleSnapshot, ScheduleSnapshotError,
    };
    use crate::{
        CommandEnvelope, CommandId, DataRevision, EntityRevision, MutationOutcome, OrderingKey,
        SchemaRevision,
    };

    struct FixturePersistence {
        snapshot: Option<ScheduleSnapshot>,
        replacement: Option<ScheduleSnapshot>,
        deleted: bool,
    }

    impl SchedulePersistence for FixturePersistence {
        fn load_schedule(
            &mut self,
            _: ScheduleId,
        ) -> Result<Option<ScheduleSnapshot>, SchedulePersistenceError> {
            Ok(self.snapshot.clone())
        }

        fn create_schedule(
            &mut self,
            _: &ScheduleSnapshot,
        ) -> Result<DataRevision, SchedulePersistenceError> {
            Err(SchedulePersistenceError::Failed)
        }

        fn replace_schedule(
            &mut self,
            snapshot: &ScheduleSnapshot,
            _: EntityRevision,
        ) -> Result<DataRevision, SchedulePersistenceError> {
            self.replacement = Some(snapshot.clone());
            Ok(DataRevision::new(2))
        }

        fn delete_schedule(
            &mut self,
            _: ScheduleId,
            _: EntityRevision,
        ) -> Result<DataRevision, SchedulePersistenceError> {
            self.deleted = true;
            Ok(DataRevision::new(2))
        }
    }

    fn recurring_snapshot() -> ScheduleSnapshot {
        ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([3; 16])),
            ScheduleRule::repeating(
                "Daily review",
                None,
                1,
                9 * 3_600,
                10 * 3_600,
                100,
                None,
                "Etc/UTC",
            )
            .expect("valid recurring fixture"),
            EntityRevision::new(0),
            UtcMicros::new(1),
        )
        .expect("matching recurring fixture")
    }

    fn delete_occurrence(scope: ScheduleEditScope, anchor_date: i32) -> CommandEnvelope<ScheduleCommand> {
        CommandEnvelope::new(
            SchemaRevision::new(1),
            CommandId::new(1),
            OrderingKey::new(1),
            Some(EntityRevision::new(0)),
            UtcMicros::new(2),
            ScheduleCommand::DeleteOccurrence {
                series_id: ScheduleSeriesId::from_bytes([3; 16]),
                anchor_date,
                scope,
            },
        )
    }

    #[test]
    fn stable_identity_must_match_one_time_or_recurring_rule_form() {
        let one_time = ScheduleRule::one_time(
            "Lunch",
            None,
            HalfOpenInterval::try_new(UtcMicros::new(10), UtcMicros::new(20))
                .expect("fixture interval is positive"),
            "Etc/UTC",
        )
        .expect("fixture one-time rule is valid");
        assert_eq!(
            ScheduleSnapshot::try_new(
                ScheduleId::Series(ScheduleSeriesId::from_bytes([1; 16])),
                one_time,
                EntityRevision::new(0),
                UtcMicros::new(0),
            ),
            Err(ScheduleSnapshotError::IdentityDoesNotMatchRule)
        );

        let recurring = ScheduleRule::repeating(
            "Daily review",
            None,
            1,
            9 * 3_600,
            10 * 3_600,
            0,
            None,
            "Etc/UTC",
        )
        .expect("fixture recurring rule is valid");
        assert_eq!(
            ScheduleSnapshot::try_new(
                ScheduleId::OneTime(OneTimeScheduleId::from_bytes([2; 16])),
                recurring,
                EntityRevision::new(0),
                UtcMicros::new(0),
            ),
            Err(ScheduleSnapshotError::IdentityDoesNotMatchRule)
        );
    }

    #[test]
    fn scoped_deletion_changes_only_the_requested_recurring_lineage() {
        let snapshot = recurring_snapshot();
        let mut service = ScheduleService::new(FixturePersistence {
            snapshot: Some(snapshot),
            replacement: None,
            deleted: false,
        });

        let outcome = service.handle(&delete_occurrence(ScheduleEditScope::OnlyThisDate, 105));
        assert!(matches!(outcome.outcome(), MutationOutcome::Confirmed(_)));
        let replacement = outcome.snapshot().expect("only-this deletion replaces the series");
        assert!(replacement.rule().is_skipped_on(105));

        let outcome = service.handle(&delete_occurrence(ScheduleEditScope::EveryOccurrence, 105));
        assert!(matches!(outcome.outcome(), MutationOutcome::Confirmed(_)));
        assert!(outcome.snapshot().is_none());
    }
}
