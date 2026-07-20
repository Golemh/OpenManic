//! Serialized authoritative writes and checkpoint recovery for the local store.

use std::path::{Path, PathBuf};

use openmanic_application::{
    ApplicationError, ApplicationPort, CatalogPersistence, CatalogPersistenceError, DataRevision,
    EntityRevision, FocusKind, FocusPersistence, FocusPersistenceError, FocusSnapshot,
    PortFailureReason, ScheduleId, SchedulePersistence, SchedulePersistenceError,
    ScheduleSnapshot, TrackingPersistenceIntent, TrackingPersistencePort,
    repeating_schedule_rules_conflict,
    schedule_rule_conflicts_with_intervals,
    TrackingPersistenceSubmit,
};
use openmanic_domain::{
    ActivityCause, ActivityInterval, ActivityState, Application, ApplicationId, Category,
    CategoryId, CategoryName, FocusSessionId, FocusSessionState, HalfOpenInterval,
    ScheduleOccurrenceException, ScheduleSegment, TrackerRunId, UtcMicros,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::repository::{database_error, read_schedule_snapshot};
use crate::{
    ConnectionConfiguration, SqliteReadSession, SqliteWriter, StorageError, StoreOpenOptions,
};

/// Metadata needed to create a durable tracker run before it receives checkpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackerRunRegistration {
    id: TrackerRunId,
    started_utc: UtcMicros,
    adapter_version: String,
}

impl TrackerRunRegistration {
    /// Creates a registration with the adapter version that produced its evidence.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the adapter version is empty after trimming.
    pub fn try_new(
        id: TrackerRunId,
        started_utc: UtcMicros,
        adapter_version: impl AsRef<str>,
    ) -> Result<Self, StorageError> {
        let adapter_version = adapter_version.as_ref().trim();
        if adapter_version.is_empty() {
            return Err(StorageError::InvalidOpenOption {
                field: "adapter_version",
            });
        }
        Ok(Self {
            id,
            started_utc,
            adapter_version: adapter_version.to_owned(),
        })
    }

    /// Returns the stable tracker run ID.
    #[must_use]
    pub const fn id(&self) -> TrackerRunId {
        self.id
    }

    /// Returns the first UTC-microsecond evidence instant for the run.
    #[must_use]
    pub const fn started_utc(&self) -> UtcMicros {
        self.started_utc
    }

    /// Returns the nonempty adapter version that generated the run's evidence.
    #[must_use]
    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }
}

/// The truthful result of recovering a checkpoint left by an unclean exit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryOutcome {
    /// The store had no open checkpoint to recover.
    NoCheckpoint,
    /// Recovery committed the trusted prior portion and any necessary unknown gap.
    Recovered {
        /// The atomic revision containing all recovery facts.
        revision: DataRevision,
        /// Whether a positive interval was closed only through its trusted checkpoint.
        closed_through_checkpoint: bool,
        /// Whether an explicit unknown gap preceded the first new observation.
        recorded_unknown_gap: bool,
    },
}

/// One crate-owned mutable writer. Exclusive mutable access serializes all mutations.
pub struct StorageWriter {
    writer: SqliteWriter,
}

impl StorageWriter {
    /// Returns the verified SQLite configuration held by this serialized writer.
    #[must_use]
    pub const fn configuration(&self) -> ConnectionConfiguration {
        self.writer.configuration()
    }

    /// Registers a new tracker run in one authoritative revision.
    ///
    /// The platform/composition boundary registers a run before passing its tracking service to
    /// [`TrackingPersistencePort`]. This prevents storage from inventing an adapter version.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the transaction cannot commit a new, unique run and revision.
    pub fn register_tracker_run(
        &mut self,
        registration: &TrackerRunRegistration,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin tracker run")?;
        let revision = next_revision(&transaction)?;
        insert_tracker_run(&transaction, registration)?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit tracker run"))?;
        Ok(revision)
    }

    /// Stores a category and advances its authoritative store revision atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the category fact or its corresponding revision cannot commit.
    pub fn upsert_category(
        &mut self,
        category: &Category,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin category mutation")?;
        let revision = next_revision(&transaction)?;
        transaction
            .execute(
                "INSERT INTO category(
                     public_id, display_name, color_spec, icon_spec, description,
                     productivity_class, created_utc_us, updated_utc_us
                 ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, ?3, ?3)
                 ON CONFLICT(public_id) DO UPDATE SET
                     display_name = excluded.display_name,
                     updated_utc_us = excluded.updated_utc_us",
                params![
                    category.id().as_bytes().as_slice(),
                    category.name().as_str(),
                    observed_at_utc.get(),
                ],
            )
            .map_err(|error| database_error(&error, "write category"))?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit category mutation"))?;
        Ok(revision)
    }

    /// Creates one category and advances the authoritative revision atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the stable ID is already present or the mutation cannot
    /// commit. Renaming is deliberately a separate operation so a create cannot silently replace
    /// an existing user category.
    pub fn create_category(
        &mut self,
        category: &Category,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin category creation")?;
        let revision = next_revision(&transaction)?;
        transaction
            .execute(
                "INSERT INTO category(
                     public_id, display_name, color_spec, icon_spec, description,
                     productivity_class, created_utc_us, updated_utc_us
                 ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, ?3, ?3)",
                params![
                    category.id().as_bytes().as_slice(),
                    category.name().as_str(),
                    observed_at_utc.get(),
                ],
            )
            .map_err(|error| database_error(&error, "create category"))?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit category creation"))?;
        Ok(revision)
    }

    /// Renames one existing category without changing application assignments.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the category no longer exists or the mutation cannot commit.
    pub fn rename_category(
        &mut self,
        category_id: CategoryId,
        name: &CategoryName,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin category rename")?;
        let revision = next_revision(&transaction)?;
        let changed = transaction
            .execute(
                "UPDATE category
                    SET display_name = ?1, updated_utc_us = ?2
                  WHERE public_id = ?3",
                params![
                    name.as_str(),
                    observed_at_utc.get(),
                    category_id.as_bytes().as_slice()
                ],
            )
            .map_err(|error| database_error(&error, "rename category"))?;
        if changed != 1 {
            return Err(StorageError::CategoryMissing);
        }
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit category rename"))?;
        Ok(revision)
    }

    /// Deletes one category, atomically returning every assigned application to Uncategorized.
    ///
    /// SQLite's `ON DELETE SET NULL` foreign key is the single authoritative assignment reset;
    /// activity rows retain their application IDs and are never rewritten.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the category no longer exists or the mutation cannot commit.
    pub fn delete_category(
        &mut self,
        category_id: CategoryId,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin category deletion")?;
        let revision = next_revision(&transaction)?;
        let changed = transaction
            .execute(
                "DELETE FROM category WHERE public_id = ?1",
                [category_id.as_bytes().as_slice()],
            )
            .map_err(|error| database_error(&error, "delete category"))?;
        if changed != 1 {
            return Err(StorageError::CategoryMissing);
        }
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit category deletion"))?;
        Ok(revision)
    }

    /// Assigns distinct existing applications to a category, or explicitly to Uncategorized.
    ///
    /// All requested applications are verified before any assignment is changed, so a stale bulk
    /// selection cannot partially mutate the catalog.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the category or an application no longer exists, or the
    /// mutation cannot commit.
    pub fn assign_applications(
        &mut self,
        application_ids: &[ApplicationId],
        category_id: Option<CategoryId>,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin application assignment")?;
        let revision = next_revision(&transaction)?;
        let category_row_id = category_id
            .map(|id| category_row_id(&transaction, id.as_bytes()))
            .transpose()?;
        let application_row_ids = application_ids
            .iter()
            .map(|id| {
                application_row_id(&transaction, id.as_bytes())?
                    .ok_or(StorageError::ApplicationMissing)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for application_row_id in application_row_ids {
            transaction
                .execute(
                    "UPDATE application SET category_id = ?1 WHERE id = ?2",
                    params![category_row_id, application_row_id],
                )
                .map_err(|error| database_error(&error, "assign application category"))?;
        }
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit application assignment"))?;
        Ok(revision)
    }

    /// Changes the exclusion policy for distinct existing applications in one revision.
    ///
    /// The tracking reducer observes this policy at its composition boundary; this mutation never
    /// rewrites historical activity rows.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if an application no longer exists or the mutation cannot commit.
    pub fn set_applications_excluded(
        &mut self,
        application_ids: &[ApplicationId],
        excluded: bool,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin application exclusion mutation")?;
        let revision = next_revision(&transaction)?;
        let application_row_ids = application_ids
            .iter()
            .map(|id| {
                application_row_id(&transaction, id.as_bytes())?
                    .ok_or(StorageError::ApplicationMissing)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for application_row_id in application_row_ids {
            transaction
                .execute(
                    "UPDATE application SET exclusion_policy = ?1 WHERE id = ?2",
                    params![i64::from(excluded), application_row_id],
                )
                .map_err(|error| database_error(&error, "set application exclusion policy"))?;
        }
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit application exclusion mutation"))?;
        Ok(revision)
    }

    /// Returns whether an existing application's future foreground observations are excluded.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the application is absent or its stored policy is invalid.
    pub fn application_is_excluded(
        &mut self,
        application_id: ApplicationId,
    ) -> Result<bool, StorageError> {
        let policy: i64 = self
            .writer
            .connection_mut()
            .query_row(
                "SELECT exclusion_policy FROM application WHERE public_id = ?1",
                [application_id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| database_error(&error, "read application exclusion policy"))?
            .ok_or(StorageError::ApplicationMissing)?;
        match policy {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StorageError::InvalidStoredValue {
                field: "application exclusion policy",
            }),
        }
    }

    /// Stores an application's current category association and observation bounds atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if a referenced category is absent or the application mutation
    /// cannot commit with its revision.
    pub fn upsert_application(
        &mut self,
        application: &Application,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin application mutation")?;
        let revision = next_revision(&transaction)?;
        let category_row_id = application
            .category_id()
            .map(|id| category_row_id(&transaction, id.as_bytes()))
            .transpose()?;
        transaction
            .execute(
                "INSERT INTO application(
                     public_id, display_name, display_name_override, category_id,
                     exclusion_policy, first_seen_utc_us, last_seen_utc_us, icon_digest
                 ) VALUES (?1, ?2, NULL, ?3, 0, ?4, ?5, NULL)
                 ON CONFLICT(public_id) DO UPDATE SET
                     display_name = excluded.display_name,
                     category_id = excluded.category_id,
                     first_seen_utc_us = MIN(application.first_seen_utc_us, excluded.first_seen_utc_us),
                     last_seen_utc_us = MAX(application.last_seen_utc_us, excluded.last_seen_utc_us)",
                params![
                    application.id().as_bytes().as_slice(),
                    application.name().as_str(),
                    category_row_id,
                    application.first_seen().get(),
                    application.last_seen().get(),
                ],
            )
            .map_err(|error| database_error(&error, "write application"))?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit application mutation"))?;
        Ok(revision)
    }

    /// Persists one complete tracking transition or checkpoint with its next data revision.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] without committing any part of the intent if the writer is busy,
    /// a tracker run/application is missing, an interval would overlap history, or the transaction
    /// cannot commit.
    pub fn persist_tracking(
        &mut self,
        intent: &TrackingPersistenceIntent,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin tracking persistence")?;
        let revision = next_revision(&transaction)?;
        let checkpoint = intent.checkpoint();
        let tracker_run_row_id = tracker_run_row_id(&transaction, checkpoint.tracker_run_id())?;
        for interval in intent.closed_intervals() {
            insert_activity_interval(&transaction, interval, tracker_run_row_id, revision, 0)?;
        }
        replace_checkpoint(&transaction, checkpoint, tracker_run_row_id, revision)?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit tracking persistence"))?;
        Ok(revision)
    }

    /// Recovers an unfinished checkpoint without extending attribution beyond trusted evidence.
    ///
    /// A recovered active interval ends exactly at `last_confirmed_utc`. Any interval between that
    /// point and the supplied next checkpoint's start is persisted as
    /// `UnknownMissing`/`CrashRecoveryGap` with no application. That checkpoint is normal,
    /// caller-supplied evidence; storage never guesses it.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the recovery boundary is invalid or any recovery fact cannot
    /// commit atomically with the old-checkpoint removal and new tracker run.
    pub fn recover_unclean_exit(
        &mut self,
        next_intent: &TrackingPersistenceIntent,
        next_run: &TrackerRunRegistration,
    ) -> Result<RecoveryOutcome, StorageError> {
        let next_checkpoint = next_intent.checkpoint();
        if !next_intent.closed_intervals().is_empty()
            || next_run.id() != next_checkpoint.tracker_run_id()
            || next_run.started_utc() != next_checkpoint.open_start_utc()
        {
            return Err(StorageError::RecoveryIntentInvalid);
        }
        let transaction = self.begin_writer_transaction("begin checkpoint recovery")?;
        let Some(checkpoint) = load_checkpoint(&transaction)? else {
            transaction
                .commit()
                .map_err(|error| database_error(&error, "commit empty checkpoint recovery"))?;
            return Ok(RecoveryOutcome::NoCheckpoint);
        };
        validate_stored_checkpoint(&checkpoint)?;
        if next_checkpoint.open_start_utc().get() < checkpoint.last_confirmed_utc_us {
            return Err(StorageError::RecoveryBoundaryBeforeCheckpoint);
        }

        let revision = next_revision(&transaction)?;
        let old_run_row_id = tracker_run_row_id_by_bytes(&transaction, &checkpoint.tracker_run_id)?;
        let closed_through_checkpoint =
            checkpoint.open_start_utc_us < checkpoint.last_confirmed_utc_us;
        if closed_through_checkpoint {
            insert_checkpoint_interval(
                &transaction,
                &checkpoint,
                old_run_row_id,
                checkpoint.open_start_utc_us,
                checkpoint.last_confirmed_utc_us,
                revision,
                2,
            )?;
        }

        let recorded_unknown_gap =
            checkpoint.last_confirmed_utc_us < next_checkpoint.open_start_utc().get();
        if recorded_unknown_gap {
            insert_unknown_recovery_gap(
                &transaction,
                old_run_row_id,
                checkpoint.last_confirmed_utc_us,
                next_checkpoint.open_start_utc().get(),
                revision,
            )?;
        }
        transaction
            .execute(
                "DELETE FROM open_activity_checkpoint WHERE singleton_id = 1",
                [],
            )
            .map_err(|error| database_error(&error, "remove recovered checkpoint"))?;
        insert_tracker_run(&transaction, next_run)?;
        let next_run_row_id = tracker_run_row_id(&transaction, next_checkpoint.tracker_run_id())?;
        replace_checkpoint(&transaction, next_checkpoint, next_run_row_id, revision)?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit checkpoint recovery"))?;
        Ok(RecoveryOutcome::Recovered {
            revision,
            closed_through_checkpoint,
            recorded_unknown_gap,
        })
    }

    fn begin_writer_transaction(
        &mut self,
        operation: &'static str,
    ) -> Result<Transaction<'_>, StorageError> {
        self.writer
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| database_error(&error, operation))
    }
}

impl TrackingPersistencePort for StorageWriter {
    fn try_persist(&mut self, intent: TrackingPersistenceIntent) -> TrackingPersistenceSubmit {
        match self.persist_tracking(&intent) {
            Ok(revision) => TrackingPersistenceSubmit::Committed(revision),
            Err(StorageError::Busy { .. }) => TrackingPersistenceSubmit::Failed(
                ApplicationError::port_failure(ApplicationPort::Command, PortFailureReason::Busy),
            ),
            Err(_) => TrackingPersistenceSubmit::Failed(ApplicationError::port_failure(
                ApplicationPort::Command,
                PortFailureReason::Failed,
            )),
        }
    }
}

impl CatalogPersistence for StorageWriter {
    fn create_category(
        &mut self,
        category: &Category,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        StorageWriter::create_category(self, category, observed_at_utc)
            .map_err(|error| catalog_persistence_error(&error))
    }

    fn rename_category(
        &mut self,
        category_id: CategoryId,
        name: &CategoryName,
        observed_at_utc: UtcMicros,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        StorageWriter::rename_category(self, category_id, name, observed_at_utc)
            .map_err(|error| catalog_persistence_error(&error))
    }

    fn delete_category(
        &mut self,
        category_id: CategoryId,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        StorageWriter::delete_category(self, category_id)
            .map_err(|error| catalog_persistence_error(&error))
    }

    fn assign_applications(
        &mut self,
        application_ids: &[ApplicationId],
        category_id: Option<CategoryId>,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        StorageWriter::assign_applications(self, application_ids, category_id)
            .map_err(|error| catalog_persistence_error(&error))
    }

    fn set_applications_excluded(
        &mut self,
        application_ids: &[ApplicationId],
        excluded: bool,
    ) -> Result<DataRevision, CatalogPersistenceError> {
        StorageWriter::set_applications_excluded(self, application_ids, excluded)
            .map_err(|error| catalog_persistence_error(&error))
    }
}

impl FocusPersistence for StorageWriter {
    fn load_focus(
        &mut self,
        session_id: FocusSessionId,
    ) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        load_focus_snapshot(self.writer.connection_mut(), Some(session_id))
            .map_err(|_| FocusPersistenceError::Failed)
    }

    fn load_active_focus(&mut self) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        load_focus_snapshot(self.writer.connection_mut(), None)
            .map_err(|_| FocusPersistenceError::Failed)
    }

    fn create_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<DataRevision, FocusPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin focus creation")
            .map_err(|_| FocusPersistenceError::Failed)?;
        let revision = next_revision(&transaction).map_err(|_| FocusPersistenceError::Failed)?;
        insert_focus_snapshot(&transaction, snapshot).map_err(|_| FocusPersistenceError::Failed)?;
        update_revision(&transaction, revision).map_err(|_| FocusPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| FocusPersistenceError::Failed)?;
        Ok(revision)
    }

    fn replace_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<(DataRevision, EntityRevision), FocusPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin focus replacement")
            .map_err(|_| FocusPersistenceError::Failed)?;
        let revision = next_revision(&transaction).map_err(|_| FocusPersistenceError::Failed)?;
        let entity_revision = snapshot
            .entity_revision()
            .get()
            .checked_add(1)
            .ok_or(FocusPersistenceError::Failed)?;
        let changed = replace_focus_snapshot(&transaction, snapshot, entity_revision)
            .map_err(|_| FocusPersistenceError::Failed)?;
        if changed != 1 {
            return Err(FocusPersistenceError::RevisionConflict);
        }
        update_revision(&transaction, revision).map_err(|_| FocusPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| FocusPersistenceError::Failed)?;
        Ok((revision, EntityRevision::new(entity_revision)))
    }
}

impl FocusPersistence for &mut StorageWriter {
    fn load_focus(
        &mut self,
        session_id: FocusSessionId,
    ) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        <StorageWriter as FocusPersistence>::load_focus(*self, session_id)
    }

    fn load_active_focus(&mut self) -> Result<Option<FocusSnapshot>, FocusPersistenceError> {
        <StorageWriter as FocusPersistence>::load_active_focus(*self)
    }

    fn create_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<DataRevision, FocusPersistenceError> {
        <StorageWriter as FocusPersistence>::create_focus(*self, snapshot)
    }

    fn replace_focus(
        &mut self,
        snapshot: &FocusSnapshot,
    ) -> Result<(DataRevision, EntityRevision), FocusPersistenceError> {
        <StorageWriter as FocusPersistence>::replace_focus(*self, snapshot)
    }
}

impl SchedulePersistence for StorageWriter {
    fn load_schedule(
        &mut self,
        schedule_id: ScheduleId,
    ) -> Result<Option<ScheduleSnapshot>, SchedulePersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin schedule scope read")
            .map_err(|_| SchedulePersistenceError::Failed)?;
        let snapshot = read_schedule_snapshot(&transaction, schedule_id)
            .map_err(|_| SchedulePersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SchedulePersistenceError::Failed)?;
        Ok(snapshot)
    }

    fn create_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin schedule creation")
            .map_err(|_| SchedulePersistenceError::Failed)?;
        if schedule_id_exists(&transaction, snapshot.id())
            .map_err(|_| SchedulePersistenceError::Failed)?
        {
            return Err(SchedulePersistenceError::Conflict);
        }
        if schedule_conflicts_with_existing_schedules(&transaction, snapshot)
            .map_err(|_| SchedulePersistenceError::Failed)?
        {
            return Err(SchedulePersistenceError::Conflict);
        }
        let revision = next_revision(&transaction).map_err(|_| SchedulePersistenceError::Failed)?;
        insert_schedule_snapshot(&transaction, snapshot)
            .map_err(|_| SchedulePersistenceError::Failed)?;
        update_revision(&transaction, revision).map_err(|_| SchedulePersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SchedulePersistenceError::Failed)?;
        Ok(revision)
    }

    fn replace_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin schedule replacement")
            .map_err(|_| SchedulePersistenceError::Failed)?;
        if schedule_revision(&transaction, snapshot.id())
            .map_err(|_| SchedulePersistenceError::Failed)?
            != Some(expected_revision)
        {
            return Err(SchedulePersistenceError::RevisionConflict);
        }
        delete_schedule_snapshot(&transaction, snapshot.id())
            .map_err(|_| SchedulePersistenceError::Failed)?;
        if schedule_conflicts_with_existing_schedules(&transaction, snapshot)
            .map_err(|_| SchedulePersistenceError::Failed)?
        {
            return Err(SchedulePersistenceError::Conflict);
        }
        let revision = next_revision(&transaction).map_err(|_| SchedulePersistenceError::Failed)?;
        insert_schedule_snapshot(&transaction, snapshot)
            .map_err(|_| SchedulePersistenceError::Failed)?;
        update_revision(&transaction, revision).map_err(|_| SchedulePersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SchedulePersistenceError::Failed)?;
        Ok(revision)
    }

    fn delete_schedule(
        &mut self,
        schedule_id: ScheduleId,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin schedule deletion")
            .map_err(|_| SchedulePersistenceError::Failed)?;
        if schedule_revision(&transaction, schedule_id)
            .map_err(|_| SchedulePersistenceError::Failed)?
            != Some(expected_revision)
        {
            return Err(SchedulePersistenceError::RevisionConflict);
        }
        let revision = next_revision(&transaction).map_err(|_| SchedulePersistenceError::Failed)?;
        delete_schedule_snapshot(&transaction, schedule_id)
            .map_err(|_| SchedulePersistenceError::Failed)?;
        update_revision(&transaction, revision).map_err(|_| SchedulePersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SchedulePersistenceError::Failed)?;
        Ok(revision)
    }
}

impl SchedulePersistence for &mut StorageWriter {
    fn load_schedule(
        &mut self,
        schedule_id: ScheduleId,
    ) -> Result<Option<ScheduleSnapshot>, SchedulePersistenceError> {
        <StorageWriter as SchedulePersistence>::load_schedule(*self, schedule_id)
    }

    fn create_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        <StorageWriter as SchedulePersistence>::create_schedule(*self, snapshot)
    }

    fn replace_schedule(
        &mut self,
        snapshot: &ScheduleSnapshot,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        <StorageWriter as SchedulePersistence>::replace_schedule(*self, snapshot, expected_revision)
    }

    fn delete_schedule(
        &mut self,
        schedule_id: ScheduleId,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SchedulePersistenceError> {
        <StorageWriter as SchedulePersistence>::delete_schedule(*self, schedule_id, expected_revision)
    }
}

fn schedule_revision(
    transaction: &Transaction<'_>,
    schedule_id: ScheduleId,
) -> Result<Option<EntityRevision>, StorageError> {
    let (table, public_id) = match schedule_id {
        ScheduleId::OneTime(id) => ("one_time_schedule", id.as_bytes()),
        ScheduleId::Series(id) => ("schedule_series", id.as_bytes()),
    };
    transaction
        .query_row(
            &format!("SELECT revision FROM {table} WHERE public_id = ?1"),
            [public_id.as_slice()],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| database_error(&error, "read schedule revision"))?
        .map(|revision| {
            u64::try_from(revision)
                .map(EntityRevision::new)
                .map_err(|_| StorageError::InvalidStoredValue {
                    field: "schedule revision",
                })
        })
        .transpose()
}

fn delete_schedule_snapshot(
    transaction: &Transaction<'_>,
    schedule_id: ScheduleId,
) -> Result<(), StorageError> {
    match schedule_id {
        ScheduleId::OneTime(id) => {
            transaction
                .execute("DELETE FROM one_time_schedule WHERE public_id = ?1", [id.as_bytes().as_slice()])
                .map_err(|error| database_error(&error, "delete one-time schedule"))?;
        }
        ScheduleId::Series(id) => {
            let series_row_id: i64 = transaction
                .query_row(
                    "SELECT id FROM schedule_series WHERE public_id = ?1",
                    [id.as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .map_err(|error| database_error(&error, "find schedule series for replacement"))?;
            transaction
                .execute("DELETE FROM schedule_exception WHERE series_id = ?1", [series_row_id])
                .map_err(|error| database_error(&error, "delete schedule exceptions"))?;
            transaction
                .execute("DELETE FROM schedule_rule_segment WHERE series_id = ?1", [series_row_id])
                .map_err(|error| database_error(&error, "delete schedule rule segments"))?;
            transaction
                .execute("DELETE FROM schedule_series WHERE id = ?1", [series_row_id])
                .map_err(|error| database_error(&error, "delete schedule series"))?;
        }
    }
    Ok(())
}

fn insert_schedule_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &ScheduleSnapshot,
) -> Result<(), StorageError> {
    match snapshot.id() {
        ScheduleId::OneTime(id) => {
            insert_one_time_schedule(transaction, snapshot, &id.as_bytes())
        }
        ScheduleId::Series(id) => insert_schedule_series(transaction, snapshot, &id.as_bytes()),
    }
}

fn schedule_id_exists(
    transaction: &Transaction<'_>,
    schedule_id: ScheduleId,
) -> Result<bool, StorageError> {
    let (table, public_id) = match schedule_id {
        ScheduleId::OneTime(id) => ("one_time_schedule", id.as_bytes()),
        ScheduleId::Series(id) => ("schedule_series", id.as_bytes()),
    };
    transaction
        .query_row(
            &format!("SELECT 1 FROM {table} WHERE public_id = ?1"),
            [public_id.as_slice()],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(|error| database_error(&error, "check schedule identity"))
}

fn schedule_conflicts_with_existing_schedules(
    transaction: &Transaction<'_>,
    snapshot: &ScheduleSnapshot,
) -> Result<bool, StorageError> {
    let mut statement = transaction
        .prepare("SELECT start_utc_us, end_utc_us FROM one_time_schedule")
        .map_err(|error| database_error(&error, "prepare one-time schedule overlap check"))?;
    let rows = statement
        .query_map([], |row| {
            let interval = HalfOpenInterval::try_new(UtcMicros::new(row.get(0)?), UtcMicros::new(row.get(1)?))
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, 0))?;
            Ok(interval)
        })
        .map_err(|error| database_error(&error, "read one-time schedule overlap check"))?;
    let intervals = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error(&error, "read one-time schedule overlap check"))?;
    if schedule_rule_conflicts_with_intervals(snapshot.rule(), &intervals).map_err(|_| {
        StorageError::InvalidStoredValue {
            field: "schedule time-zone overlap validation",
        }
    })? {
        return Ok(true);
    }

    let series_rules = load_schedule_series_rules(transaction)?;
    if let Some(candidate_interval) = snapshot.rule().one_time_interval() {
        for rule in &series_rules {
            if schedule_rule_conflicts_with_intervals(rule, &[candidate_interval]).map_err(|_| {
                StorageError::InvalidStoredValue {
                    field: "schedule time-zone overlap validation",
                }
            })? {
                return Ok(true);
            }
        }
    } else {
        for rule in &series_rules {
            if repeating_schedule_rules_conflict(snapshot.rule(), rule).map_err(|_| {
                StorageError::InvalidStoredValue {
                    field: "schedule time-zone overlap validation",
                }
            })? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn load_schedule_series_rules(
    transaction: &Transaction<'_>,
) -> Result<Vec<openmanic_domain::ScheduleRule>, StorageError> {
    let mut series_statement = transaction
        .prepare("SELECT id FROM schedule_series")
        .map_err(|error| database_error(&error, "prepare schedule-series overlap check"))?;
    let series_ids = series_statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| database_error(&error, "read schedule-series overlap check"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error(&error, "read schedule-series overlap check"))?;

    series_ids
        .into_iter()
        .map(|series_id| load_schedule_series_rule(transaction, series_id))
        .collect()
}

pub(crate) fn load_schedule_series_rule(
    transaction: &Transaction<'_>,
    series_id: i64,
) -> Result<openmanic_domain::ScheduleRule, StorageError> {
    let segments = load_schedule_segments(transaction, series_id)?;
    let exceptions = load_schedule_exceptions(transaction, series_id)?;
    openmanic_domain::ScheduleRule::try_restore_repeating_with_exceptions(segments, exceptions)
        .map_err(|_| StorageError::InvalidStoredValue {
            field: "stored schedule series",
        })
}

fn load_schedule_segments(
    transaction: &Transaction<'_>,
    series_id: i64,
) -> Result<Vec<ScheduleSegment>, StorageError> {
    let mut segment_statement = transaction
        .prepare(
            "SELECT effective_start_date, effective_end_date, weekday_mask,
                    start_second_of_day, end_second_of_day, time_zone_id, label, category.public_id
               FROM schedule_rule_segment
          LEFT JOIN category ON category.id = schedule_rule_segment.category_id
              WHERE series_id = ?1
              ORDER BY effective_start_date",
        )
        .map_err(|error| database_error(&error, "prepare stored schedule segments"))?;
    let raw_segments = segment_statement
        .query_map([series_id], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, Option<i32>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<Vec<u8>>>(7)?,
            ))
        })
        .map_err(|error| database_error(&error, "read stored schedule segments"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error(&error, "read stored schedule segments"))?;
    let segments = raw_segments
        .into_iter()
        .map(
            |(start, end, weekday_mask, start_second, end_second, zone, label, category_id)| {
                ScheduleSegment::try_new(
                    start,
                    end,
                    u8::try_from(weekday_mask).map_err(|_| StorageError::InvalidStoredValue {
                        field: "schedule weekday mask",
                    })?,
                    u32::try_from(start_second).map_err(|_| StorageError::InvalidStoredValue {
                        field: "schedule start second",
                    })?,
                    u32::try_from(end_second).map_err(|_| StorageError::InvalidStoredValue {
                        field: "schedule end second",
                    })?,
                    zone,
                    label,
                    category_id
                        .map(|value| {
                            value.try_into().map_err(|_| StorageError::InvalidStoredValue {
                                field: "schedule segment category ID",
                            })
                        })
                        .transpose()?
                        .map(CategoryId::from_bytes),
                )
                .map_err(|_| StorageError::InvalidStoredValue {
                    field: "stored schedule rule segment",
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    Ok(segments)
}

fn load_schedule_exceptions(
    transaction: &Transaction<'_>,
    series_id: i64,
) -> Result<Vec<ScheduleOccurrenceException>, StorageError> {
    let mut exception_statement = transaction
        .prepare(
            "SELECT anchor_local_date, kind, override_start_utc_us, override_end_utc_us,
                    start_boundary_resolution, end_boundary_resolution
               FROM schedule_exception
              WHERE series_id = ?1
              ORDER BY anchor_local_date",
        )
        .map_err(|error| database_error(&error, "prepare stored schedule exceptions"))?;
    let raw_exceptions = exception_statement
        .query_map([series_id], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| database_error(&error, "read stored schedule exceptions"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| database_error(&error, "read stored schedule exceptions"))?;
    raw_exceptions
        .into_iter()
        .map(schedule_occurrence_exception_from_row)
        .collect()
}

fn schedule_occurrence_exception_from_row(
    row: (i32, i64, Option<i64>, Option<i64>, i64, i64),
) -> Result<ScheduleOccurrenceException, StorageError> {
    let (anchor_date, kind, start, end, start_resolution, end_resolution) = row;
    match kind {
        0 => Ok(ScheduleOccurrenceException::Skip { anchor_date }),
        1 => {
            let interval = HalfOpenInterval::try_new(
                UtcMicros::new(start.ok_or(StorageError::InvalidStoredValue {
                    field: "schedule exception override start",
                })?),
                UtcMicros::new(end.ok_or(StorageError::InvalidStoredValue {
                    field: "schedule exception override end",
                })?),
            )
            .map_err(|_| StorageError::InvalidStoredValue {
                field: "schedule exception override interval",
            })?;
            let (start_after_gap, start_earlier_fold) =
                boundary_resolution_flags(start_resolution)?;
            let (end_after_gap, end_earlier_fold) = boundary_resolution_flags(end_resolution)?;
            Ok(ScheduleOccurrenceException::Override {
                anchor_date,
                interval,
                start_after_gap,
                start_earlier_fold,
                end_after_gap,
                end_earlier_fold,
            })
        }
        _ => Err(StorageError::InvalidStoredValue {
            field: "schedule exception kind",
        }),
    }
}

fn boundary_resolution_flags(code: i64) -> Result<(bool, bool), StorageError> {
    match code {
        0 => Ok((false, false)),
        1 => Ok((true, false)),
        2 => Ok((false, true)),
        _ => Err(StorageError::InvalidStoredValue {
            field: "schedule exception boundary resolution",
        }),
    }
}

fn insert_one_time_schedule(
    transaction: &Transaction<'_>,
    snapshot: &ScheduleSnapshot,
    public_id: &[u8; 16],
) -> Result<(), StorageError> {
    let rule = snapshot.rule();
    let interval = rule.one_time_interval().ok_or(StorageError::InvalidStoredValue {
        field: "one-time schedule rule",
    })?;
    let created_zone_id = rule.created_zone_id().ok_or(StorageError::InvalidStoredValue {
        field: "one-time schedule creation zone",
    })?;
    let category_row_id = rule
        .category_id()
        .map(|category_id| category_row_id(transaction, category_id.as_bytes()))
        .transpose()?;
    let entity_revision = i64::try_from(snapshot.entity_revision().get()).map_err(|_| {
        StorageError::InvalidStoredValue {
            field: "schedule revision",
        }
    })?;
    transaction
        .execute(
            "INSERT INTO one_time_schedule(
                 public_id, label, category_id, start_utc_us, end_utc_us, created_zone_id,
                 created_utc_us, updated_utc_us, revision
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
            params![
                public_id.as_slice(),
                rule.label(),
                category_row_id,
                interval.start().get(),
                interval.end().get(),
                created_zone_id,
                snapshot.created_at_utc().get(),
                entity_revision,
            ],
        )
        .map_err(|error| database_error(&error, "create one-time schedule"))?;
    Ok(())
}

fn insert_schedule_series(
    transaction: &Transaction<'_>,
    snapshot: &ScheduleSnapshot,
    public_id: &[u8; 16],
) -> Result<(), StorageError> {
    let entity_revision = i64::try_from(snapshot.entity_revision().get()).map_err(|_| {
        StorageError::InvalidStoredValue {
            field: "schedule revision",
        }
    })?;
    transaction
        .execute(
            "INSERT INTO schedule_series(public_id, created_utc_us, deleted_utc_us, revision)
             VALUES (?1, ?2, NULL, ?3)",
            params![
                public_id.as_slice(),
                snapshot.created_at_utc().get(),
                entity_revision,
            ],
        )
        .map_err(|error| database_error(&error, "create schedule series"))?;
    let series_row_id: i64 = transaction
        .query_row(
            "SELECT id FROM schedule_series WHERE public_id = ?1",
            [public_id.as_slice()],
            |row| row.get(0),
        )
        .map_err(|error| database_error(&error, "find created schedule series"))?;
    for segment in snapshot.rule().segments() {
        let category_row_id = segment
            .category_id()
            .map(|category_id| category_row_id(transaction, category_id.as_bytes()))
            .transpose()?;
        transaction
            .execute(
                "INSERT INTO schedule_rule_segment(
                     series_id, effective_start_date, effective_end_date, weekday_mask,
                     start_second_of_day, end_second_of_day, end_day_offset, time_zone_id,
                     label, category_id, created_utc_us, revision
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    series_row_id,
                    segment.effective_start_date(),
                    segment.effective_end_date(),
                    i64::from(segment.weekday_mask()),
                    i64::from(segment.start_second_of_day()),
                    i64::from(segment.end_second_of_day()),
                    i64::from(segment.end_day_offset()),
                    segment.time_zone_id(),
                    segment.label(),
                    category_row_id,
                    snapshot.created_at_utc().get(),
                    entity_revision,
                ],
            )
            .map_err(|error| database_error(&error, "create schedule rule segment"))?;
    }
    for exception in snapshot.rule().exceptions() {
        insert_schedule_exception(
            transaction,
            series_row_id,
            snapshot.rule(),
            exception,
            entity_revision,
        )?;
    }
    Ok(())
}

fn insert_schedule_exception(
    transaction: &Transaction<'_>,
    series_row_id: i64,
    rule: &openmanic_domain::ScheduleRule,
    exception: ScheduleOccurrenceException,
    entity_revision: i64,
) -> Result<(), StorageError> {
    let (anchor_date, kind, start, end, start_resolution, end_resolution) = match exception {
        ScheduleOccurrenceException::Skip { anchor_date } => (anchor_date, 0, None, None, 0, 0),
        ScheduleOccurrenceException::Override {
            anchor_date,
            interval,
            start_after_gap,
            start_earlier_fold,
            end_after_gap,
            end_earlier_fold,
        } => (
            anchor_date,
            1,
            Some(interval.start().get()),
            Some(interval.end().get()),
            boundary_resolution_code(start_after_gap, start_earlier_fold)?,
            boundary_resolution_code(end_after_gap, end_earlier_fold)?,
        ),
    };
    let resolved_zone_id = rule.time_zone_for_anchor_date(anchor_date).ok_or(
        StorageError::InvalidStoredValue {
            field: "schedule exception rule segment",
        },
    )?;
    transaction
        .execute(
            "INSERT INTO schedule_exception(
                 series_id, anchor_local_date, kind, override_start_utc_us, override_end_utc_us,
                 label_override, category_id_override, resolved_zone_id, revision,
                 start_boundary_resolution, end_boundary_resolution
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, ?7, ?8, ?9)",
            params![
                series_row_id,
                anchor_date,
                kind,
                start,
                end,
                resolved_zone_id,
                entity_revision,
                start_resolution,
                end_resolution,
            ],
        )
        .map_err(|error| database_error(&error, "create schedule exception"))?;
    Ok(())
}

fn boundary_resolution_code(after_gap: bool, earlier_fold: bool) -> Result<i64, StorageError> {
    match (after_gap, earlier_fold) {
        (false, false) => Ok(0),
        (true, false) => Ok(1),
        (false, true) => Ok(2),
        (true, true) => Err(StorageError::InvalidStoredValue {
            field: "schedule exception boundary resolution",
        }),
    }
}

fn load_focus_snapshot(
    connection: &mut rusqlite::Connection,
    session_id: Option<FocusSessionId>,
) -> Result<Option<FocusSnapshot>, StorageError> {
    let query = "SELECT focus_session.public_id, focus_session.kind, focus_session.label,
                        category.public_id, focus_session.intended_duration_us, focus_session.state,
                        focus_session.planned_start_utc_us, focus_session.planned_end_utc_us,
                        focus_session.actual_start_utc_us, focus_session.deadline_utc_us,
                        focus_session.paused_remaining_us, focus_session.completed_utc_us,
                        focus_session.cancelled_utc_us, focus_session.revision
                   FROM focus_session LEFT JOIN category ON category.id = focus_session.category_id";
    let row = match session_id {
        Some(session_id) => connection
            .query_row(
                &format!("{query} WHERE focus_session.public_id = ?1"),
                [session_id.as_bytes().as_slice()],
                focus_snapshot_row,
            )
            .optional(),
        None => connection
            .query_row(
                &format!(
                    "{query} WHERE focus_session.state IN (2, 3) ORDER BY focus_session.id LIMIT 1"
                ),
                [],
                focus_snapshot_row,
            )
            .optional(),
    }
    .map_err(|error| database_error(&error, "load focus snapshot"))?;
    Ok(row)
}

fn focus_snapshot_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FocusSnapshot> {
    let id = row.get::<_, Vec<u8>>(0)?;
    let kind = match row.get::<_, i64>(1)? {
        0 => FocusKind::Focus,
        1 => FocusKind::ShortBreak,
        _ => return Err(rusqlite::Error::IntegralValueOutOfRange(1, 0)),
    };
    let category_id = row
        .get::<_, Option<Vec<u8>>>(3)?
        .map(fixed_focus_id)
        .transpose()
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, 0))?
        .map(CategoryId::from_bytes);
    let state = focus_state_from_columns((
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
    ))
    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, 0))?;
    FocusSnapshot::try_restore(
        FocusSessionId::from_bytes(
            fixed_focus_id(id).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, 0))?,
        ),
        kind,
        row.get(2)?,
        row.get(4)?,
        category_id,
        state,
        EntityRevision::new(
            u64::try_from(row.get::<_, i64>(13)?)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(13, 0))?,
        ),
    )
    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, 0))
}

fn fixed_focus_id(value: Vec<u8>) -> Result<[u8; 16], StorageError> {
    value
        .try_into()
        .map_err(|_| StorageError::InvalidStoredValue {
            field: "focus stable ID",
        })
}

fn insert_focus_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &FocusSnapshot,
) -> Result<(), StorageError> {
    let fields = focus_fields(transaction, snapshot)?;
    transaction.execute(
        "INSERT INTO focus_session(public_id, kind, state, label, category_id, planned_start_utc_us, planned_end_utc_us, intended_duration_us, actual_start_utc_us, deadline_utc_us, paused_remaining_us, completed_utc_us, cancelled_utc_us, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![snapshot.session_id().as_bytes().as_slice(), fields.kind, fields.state, snapshot.label(), fields.category_row_id, fields.planned_start, fields.planned_end, snapshot.session().intended_duration_us(), fields.actual_start, fields.deadline, fields.paused_remaining, fields.completed, fields.cancelled, i64::try_from(snapshot.entity_revision().get()).map_err(|_| StorageError::InvalidStoredValue { field: "focus revision" })?],
    ).map_err(|error| database_error(&error, "create focus session"))?;
    Ok(())
}

fn replace_focus_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &FocusSnapshot,
    entity_revision: u64,
) -> Result<usize, StorageError> {
    let fields = focus_fields(transaction, snapshot)?;
    transaction.execute(
        "UPDATE focus_session SET kind = ?1, state = ?2, label = ?3, category_id = ?4, planned_start_utc_us = ?5, planned_end_utc_us = ?6, intended_duration_us = ?7, actual_start_utc_us = ?8, deadline_utc_us = ?9, paused_remaining_us = ?10, completed_utc_us = ?11, cancelled_utc_us = ?12, revision = ?13 WHERE public_id = ?14 AND revision = ?15",
        params![fields.kind, fields.state, snapshot.label(), fields.category_row_id, fields.planned_start, fields.planned_end, snapshot.session().intended_duration_us(), fields.actual_start, fields.deadline, fields.paused_remaining, fields.completed, fields.cancelled, i64::try_from(entity_revision).map_err(|_| StorageError::InvalidStoredValue { field: "focus revision" })?, snapshot.session_id().as_bytes().as_slice(), i64::try_from(snapshot.entity_revision().get()).map_err(|_| StorageError::InvalidStoredValue { field: "focus revision" })?],
    ).map_err(|error| database_error(&error, "replace focus session"))
}

type FocusStateColumns = (
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

struct FocusFields {
    kind: i64,
    state: i64,
    category_row_id: Option<i64>,
    planned_start: Option<i64>,
    planned_end: Option<i64>,
    actual_start: Option<i64>,
    deadline: Option<i64>,
    paused_remaining: Option<i64>,
    completed: Option<i64>,
    cancelled: Option<i64>,
}

fn focus_fields(
    transaction: &Transaction<'_>,
    snapshot: &FocusSnapshot,
) -> Result<FocusFields, StorageError> {
    let (
        state,
        planned_start,
        planned_end,
        actual_start,
        deadline,
        paused_remaining,
        completed,
        cancelled,
    ) = focus_state_columns(snapshot.session().state());
    let category_row_id = snapshot
        .session()
        .category_id()
        .map(|id| category_row_id(transaction, id.as_bytes()))
        .transpose()?;
    Ok(FocusFields {
        kind: match snapshot.kind() {
            FocusKind::Focus => 0,
            FocusKind::ShortBreak => 1,
        },
        state,
        category_row_id,
        planned_start,
        planned_end,
        actual_start,
        deadline,
        paused_remaining,
        completed,
        cancelled,
    })
}

fn focus_state_columns(state: FocusSessionState) -> FocusStateColumns {
    match state {
        FocusSessionState::Ready => (0, None, None, None, None, None, None, None),
        FocusSessionState::Planned {
            planned_start,
            planned_end,
        } => (
            1,
            Some(planned_start.get()),
            Some(planned_end.get()),
            None,
            None,
            None,
            None,
            None,
        ),
        FocusSessionState::Running {
            started_at,
            deadline,
        } => (
            2,
            None,
            None,
            Some(started_at.get()),
            Some(deadline.get()),
            None,
            None,
            None,
        ),
        FocusSessionState::Paused {
            started_at,
            remaining_us,
        } => (
            3,
            None,
            None,
            Some(started_at.get()),
            None,
            Some(remaining_us),
            None,
            None,
        ),
        FocusSessionState::Completed {
            started_at,
            completed_at,
        } => (
            4,
            None,
            None,
            Some(started_at.get()),
            None,
            None,
            Some(completed_at.get()),
            None,
        ),
        FocusSessionState::Cancelled {
            started_at,
            cancelled_at,
        } => (
            5,
            None,
            None,
            Some(started_at.get()),
            None,
            None,
            None,
            Some(cancelled_at.get()),
        ),
    }
}

fn focus_state_from_columns(columns: FocusStateColumns) -> Result<FocusSessionState, StorageError> {
    match columns {
        (0, None, None, None, None, None, None, None) => Ok(FocusSessionState::Ready),
        (1, Some(start), Some(end), None, None, None, None, None) => {
            Ok(FocusSessionState::Planned {
                planned_start: UtcMicros::new(start),
                planned_end: UtcMicros::new(end),
            })
        }
        (2, None, None, Some(start), Some(deadline), None, None, None) => {
            Ok(FocusSessionState::Running {
                started_at: UtcMicros::new(start),
                deadline: UtcMicros::new(deadline),
            })
        }
        (3, None, None, Some(start), None, Some(remaining), None, None) => {
            Ok(FocusSessionState::Paused {
                started_at: UtcMicros::new(start),
                remaining_us: remaining,
            })
        }
        (4, None, None, Some(start), None, None, Some(completed), None) => {
            Ok(FocusSessionState::Completed {
                started_at: UtcMicros::new(start),
                completed_at: UtcMicros::new(completed),
            })
        }
        (5, None, None, Some(start), None, None, None, Some(cancelled)) => {
            Ok(FocusSessionState::Cancelled {
                started_at: UtcMicros::new(start),
                cancelled_at: UtcMicros::new(cancelled),
            })
        }
        _ => Err(StorageError::InvalidStoredValue {
            field: "focus session state",
        }),
    }
}

fn catalog_persistence_error(error: &StorageError) -> CatalogPersistenceError {
    match error {
        StorageError::CategoryMissing | StorageError::ApplicationMissing => {
            CatalogPersistenceError::NotFound
        }
        _ => CatalogPersistenceError::Failed,
    }
}

/// A local store facade that creates one serialized writer and short read sessions.
pub struct SqliteStore {
    database_path: PathBuf,
    writer: StorageWriter,
}

impl SqliteStore {
    /// Opens a fully migrated store and its sole crate-owned writer connection.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the writer cannot open, configure, verify, or migrate the
    /// requested database.
    pub fn open(path: &Path, options: &StoreOpenOptions) -> Result<Self, StorageError> {
        Ok(Self {
            database_path: path.to_path_buf(),
            writer: StorageWriter {
                writer: SqliteWriter::open(path, options)?,
            },
        })
    }

    /// Returns the exclusive serialized writer owned by this store facade.
    #[must_use]
    pub fn writer(&mut self) -> &mut StorageWriter {
        &mut self.writer
    }

    /// Opens one short, query-only reader session with no shared SQLite connection mutex.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the reader cannot open the already migrated local store.
    pub fn open_read_session(&self) -> Result<SqliteReadSession, StorageError> {
        SqliteReadSession::open(&self.database_path)
    }
}

fn insert_tracker_run(
    transaction: &Transaction<'_>,
    registration: &TrackerRunRegistration,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO tracker_run(
                 public_id, started_utc_us, ended_utc_us, clean_end, platform_session_marker,
                 adapter_version, end_evidence
             ) VALUES (?1, ?2, NULL, 0, NULL, ?3, NULL)",
            params![
                registration.id().as_bytes().as_slice(),
                registration.started_utc().get(),
                registration.adapter_version(),
            ],
        )
        .map_err(|error| database_error(&error, "register tracker run"))?;
    Ok(())
}

fn next_revision(transaction: &Transaction<'_>) -> Result<DataRevision, StorageError> {
    let current: i64 = transaction
        .query_row(
            "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| database_error(&error, "read current data revision"))?;
    let current = u64::try_from(current).map_err(|_| StorageError::InvalidStoredValue {
        field: "data revision",
    })?;
    current
        .checked_add(1)
        .map(DataRevision::new)
        .ok_or(StorageError::RevisionOverflow)
}

fn update_revision(
    transaction: &Transaction<'_>,
    revision: DataRevision,
) -> Result<(), StorageError> {
    let revision = i64::try_from(revision.get()).map_err(|_| StorageError::RevisionOverflow)?;
    let changed = transaction
        .execute(
            "UPDATE store_metadata SET data_revision = ?1 WHERE singleton_id = 1",
            [revision],
        )
        .map_err(|error| database_error(&error, "advance data revision"))?;
    if changed != 1 {
        return Err(StorageError::InvalidStoredValue {
            field: "store metadata",
        });
    }
    Ok(())
}

fn tracker_run_row_id(
    transaction: &Transaction<'_>,
    tracker_run_id: TrackerRunId,
) -> Result<i64, StorageError> {
    tracker_run_row_id_by_bytes(transaction, &tracker_run_id.as_bytes())
}

fn tracker_run_row_id_by_bytes(
    transaction: &Transaction<'_>,
    tracker_run_id: &[u8; 16],
) -> Result<i64, StorageError> {
    transaction
        .query_row(
            "SELECT id FROM tracker_run WHERE public_id = ?1",
            [tracker_run_id.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| database_error(&error, "find tracker run"))?
        .ok_or(StorageError::TrackerRunMissing)
}

fn category_row_id(
    transaction: &Transaction<'_>,
    category_id: [u8; 16],
) -> Result<i64, StorageError> {
    transaction
        .query_row(
            "SELECT id FROM category WHERE public_id = ?1",
            [category_id.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| database_error(&error, "find application category"))?
        .ok_or(StorageError::CategoryMissing)
}

fn application_row_id(
    transaction: &Transaction<'_>,
    application_id: [u8; 16],
) -> Result<Option<i64>, StorageError> {
    transaction
        .query_row(
            "SELECT id FROM application WHERE public_id = ?1",
            [application_id.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| database_error(&error, "find activity application"))
}

fn insert_activity_interval(
    transaction: &Transaction<'_>,
    interval: &ActivityInterval,
    tracker_run_row_id: i64,
    revision: DataRevision,
    origin: i64,
) -> Result<(), StorageError> {
    let application_row_id = interval
        .application_id()
        .map(|id| application_row_id(transaction, id.as_bytes()))
        .transpose()?
        .flatten();
    if interval.state() == ActivityState::Active && application_row_id.is_none() {
        return Err(StorageError::InvalidStoredValue {
            field: "activity application reference",
        });
    }
    insert_activity_row(
        transaction,
        ActivityRow {
            tracker_run_row_id,
            start_utc_us: interval.range().start().get(),
            end_utc_us: interval.range().end().get(),
            state: activity_state_code(interval.state()),
            cause: activity_cause_code(interval.cause()),
            application_row_id,
            origin,
            uncertainty_us: 0,
            revision,
        },
    )
}

fn replace_checkpoint(
    transaction: &Transaction<'_>,
    checkpoint: openmanic_application::TrackingCheckpoint,
    tracker_run_row_id: i64,
    revision: DataRevision,
) -> Result<(), StorageError> {
    let application_row_id = checkpoint
        .application_id()
        .map(|id| application_row_id(transaction, id.as_bytes()))
        .transpose()?
        .flatten();
    if checkpoint.state() == ActivityState::Active && application_row_id.is_none() {
        return Err(StorageError::InvalidStoredValue {
            field: "checkpoint application reference",
        });
    }
    transaction
        .execute(
            "INSERT INTO open_activity_checkpoint(
                 singleton_id, tracker_run_id, open_start_utc_us, last_confirmed_utc_us,
                 state, cause, application_id, platform_sequence, checkpoint_revision
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(singleton_id) DO UPDATE SET
                 tracker_run_id = excluded.tracker_run_id,
                 open_start_utc_us = excluded.open_start_utc_us,
                 last_confirmed_utc_us = excluded.last_confirmed_utc_us,
                 state = excluded.state,
                 cause = excluded.cause,
                 application_id = excluded.application_id,
                 platform_sequence = excluded.platform_sequence,
                 checkpoint_revision = excluded.checkpoint_revision",
            params![
                tracker_run_row_id,
                checkpoint.open_start_utc().get(),
                checkpoint.last_confirmed_utc().get(),
                activity_state_code(checkpoint.state()),
                activity_cause_code(checkpoint.cause()),
                application_row_id,
                nonnegative_i64(checkpoint.platform_sequence(), "platform sequence")?,
                nonnegative_i64(revision.get(), "checkpoint revision")?,
            ],
        )
        .map_err(|error| database_error(&error, "replace activity checkpoint"))?;
    Ok(())
}

#[derive(Clone, Copy)]
struct ActivityRow {
    tracker_run_row_id: i64,
    start_utc_us: i64,
    end_utc_us: i64,
    state: i64,
    cause: i64,
    application_row_id: Option<i64>,
    origin: i64,
    uncertainty_us: i64,
    revision: DataRevision,
}

fn insert_activity_row(
    transaction: &Transaction<'_>,
    row: ActivityRow,
) -> Result<(), StorageError> {
    ensure_no_overlap(transaction, row.start_utc_us, row.end_utc_us)?;
    transaction
        .execute(
            "INSERT INTO activity_interval(
                 tracker_run_id, start_utc_us, end_utc_us, state, cause, application_id,
                 origin, uncertainty_us, source_revision
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                row.tracker_run_row_id,
                row.start_utc_us,
                row.end_utc_us,
                row.state,
                row.cause,
                row.application_row_id,
                row.origin,
                row.uncertainty_us,
                nonnegative_i64(row.revision.get(), "activity source revision")?,
            ],
        )
        .map_err(|error| database_error(&error, "insert activity interval"))?;
    Ok(())
}

fn ensure_no_overlap(
    transaction: &Transaction<'_>,
    start_utc_us: i64,
    end_utc_us: i64,
) -> Result<(), StorageError> {
    let overlaps: i64 = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM activity_interval
                  WHERE start_utc_us < ?1 AND end_utc_us > ?2
             )",
            params![end_utc_us, start_utc_us],
            |row| row.get(0),
        )
        .map_err(|error| database_error(&error, "check activity overlap"))?;
    if overlaps != 0 {
        return Err(StorageError::InvalidStoredValue {
            field: "overlapping activity interval",
        });
    }
    Ok(())
}

fn nonnegative_i64(value: u64, field: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidStoredValue { field })
}

fn activity_state_code(state: ActivityState) -> i64 {
    match state {
        ActivityState::Active => 0,
        ActivityState::Idle => 1,
        ActivityState::PausedByUser => 2,
        ActivityState::Excluded => 3,
        ActivityState::Unavailable => 4,
        ActivityState::PoweredOff => 5,
        ActivityState::UnknownMissing => 6,
    }
}

fn activity_cause_code(cause: ActivityCause) -> i64 {
    match cause {
        ActivityCause::ForegroundApplication => 0,
        ActivityCause::IdleThreshold => 1,
        ActivityCause::UserPause => 2,
        ActivityCause::ApplicationExcluded => 3,
        ActivityCause::SessionLocked => 4,
        ActivityCause::SessionDisconnected => 5,
        ActivityCause::SystemSuspended => 6,
        ActivityCause::AdapterStarting => 7,
        ActivityCause::AdapterPermissionLost => 8,
        ActivityCause::AdapterFailure => 9,
        ActivityCause::EvidenceQueueOverflow => 10,
        ActivityCause::ConfirmedShutdown => 11,
        ActivityCause::CrashRecoveryGap => 12,
        ActivityCause::ImportedUnknown => 13,
        ActivityCause::ClockDiscontinuity => 14,
    }
}

struct StoredCheckpoint {
    tracker_run_id: [u8; 16],
    open_start_utc_us: i64,
    last_confirmed_utc_us: i64,
    state: i64,
    cause: i64,
    application_row_id: Option<i64>,
}

fn load_checkpoint(
    transaction: &Transaction<'_>,
) -> Result<Option<StoredCheckpoint>, StorageError> {
    transaction
        .query_row(
            "SELECT tracker_run.public_id, open_start_utc_us, last_confirmed_utc_us,
                    state, cause, application_id
               FROM open_activity_checkpoint
               JOIN tracker_run ON tracker_run.id = open_activity_checkpoint.tracker_run_id
              WHERE singleton_id = 1",
            [],
            |row| {
                let tracker_run_id: Vec<u8> = row.get(0)?;
                let tracker_run_id = tracker_run_id
                    .try_into()
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, 16_i64))?;
                Ok(StoredCheckpoint {
                    tracker_run_id,
                    open_start_utc_us: row.get(1)?,
                    last_confirmed_utc_us: row.get(2)?,
                    state: row.get(3)?,
                    cause: row.get(4)?,
                    application_row_id: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| database_error(&error, "load activity checkpoint"))
}

fn insert_checkpoint_interval(
    transaction: &Transaction<'_>,
    checkpoint: &StoredCheckpoint,
    tracker_run_row_id: i64,
    start_utc_us: i64,
    end_utc_us: i64,
    revision: DataRevision,
    origin: i64,
) -> Result<(), StorageError> {
    validate_stored_checkpoint(checkpoint)?;
    insert_activity_row(
        transaction,
        ActivityRow {
            tracker_run_row_id,
            start_utc_us,
            end_utc_us,
            state: checkpoint.state,
            cause: checkpoint.cause,
            application_row_id: checkpoint.application_row_id,
            origin,
            uncertainty_us: 0,
            revision,
        },
    )
}

fn validate_stored_checkpoint(checkpoint: &StoredCheckpoint) -> Result<(), StorageError> {
    let state = stored_activity_state(checkpoint.state)?;
    let cause = stored_activity_cause(checkpoint.cause)?;
    if state == ActivityState::PoweredOff || cause == ActivityCause::ConfirmedShutdown {
        return Err(StorageError::InvalidStoredValue {
            field: "open activity checkpoint evidence",
        });
    }
    if (state == ActivityState::Active) != checkpoint.application_row_id.is_some() {
        return Err(StorageError::InvalidStoredValue {
            field: "open activity checkpoint application",
        });
    }
    Ok(())
}

fn stored_activity_state(value: i64) -> Result<ActivityState, StorageError> {
    match value {
        0 => Ok(ActivityState::Active),
        1 => Ok(ActivityState::Idle),
        2 => Ok(ActivityState::PausedByUser),
        3 => Ok(ActivityState::Excluded),
        4 => Ok(ActivityState::Unavailable),
        5 => Ok(ActivityState::PoweredOff),
        6 => Ok(ActivityState::UnknownMissing),
        _ => Err(StorageError::InvalidStoredValue {
            field: "open activity checkpoint state",
        }),
    }
}

fn stored_activity_cause(value: i64) -> Result<ActivityCause, StorageError> {
    match value {
        0 => Ok(ActivityCause::ForegroundApplication),
        1 => Ok(ActivityCause::IdleThreshold),
        2 => Ok(ActivityCause::UserPause),
        3 => Ok(ActivityCause::ApplicationExcluded),
        4 => Ok(ActivityCause::SessionLocked),
        5 => Ok(ActivityCause::SessionDisconnected),
        6 => Ok(ActivityCause::SystemSuspended),
        7 => Ok(ActivityCause::AdapterStarting),
        8 => Ok(ActivityCause::AdapterPermissionLost),
        9 => Ok(ActivityCause::AdapterFailure),
        10 => Ok(ActivityCause::EvidenceQueueOverflow),
        11 => Ok(ActivityCause::ConfirmedShutdown),
        12 => Ok(ActivityCause::CrashRecoveryGap),
        13 => Ok(ActivityCause::ImportedUnknown),
        14 => Ok(ActivityCause::ClockDiscontinuity),
        _ => Err(StorageError::InvalidStoredValue {
            field: "open activity checkpoint cause",
        }),
    }
}

fn insert_unknown_recovery_gap(
    transaction: &Transaction<'_>,
    tracker_run_row_id: i64,
    start_utc_us: i64,
    end_utc_us: i64,
    revision: DataRevision,
) -> Result<(), StorageError> {
    insert_activity_row(
        transaction,
        ActivityRow {
            tracker_run_row_id,
            start_utc_us,
            end_utc_us,
            state: activity_state_code(ActivityState::UnknownMissing),
            cause: activity_cause_code(ActivityCause::CrashRecoveryGap),
            application_row_id: None,
            origin: 2,
            uncertainty_us: 0,
            revision,
        },
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use openmanic_application::{
        CommandEnvelope, CommandId, EntityRevision, FocusCommand, FocusKind,
        FocusNotificationError, FocusNotificationPort, FocusPersistence, FocusService,
        FocusSnapshot, OrderingKey, ScheduleId, SchedulePersistence, SchedulePersistenceError,
        ScheduleSnapshot, SchemaRevision, TrackingCheckpoint, TrackingPersistenceIntent,
        TrackingPersistencePort, TrackingPersistenceSubmit,
    };
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, Application,
        ApplicationId, ApplicationName, Category, CategoryId, CategoryName, FocusSessionId,
        FocusSessionState, HalfOpenInterval, OneTimeScheduleId, ScheduleRule, ScheduleSeriesId,
        TrackerRunId, UtcMicros,
    };
    use rusqlite::Connection;

    use super::{
        RecoveryOutcome, SqliteStore, StorageWriter, TrackerRunRegistration, activity_cause_code,
        activity_state_code,
    };
    use crate::{JournalMode, StorageError, StoreOpenOptions, SynchronousMode};

    static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDatabase {
        path: PathBuf,
    }

    impl TemporaryDatabase {
        fn new(case_name: &str) -> Self {
            let sequence = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            let filename = format!(
                "openmanic-om220-{case_name}-{}-{sequence}.sqlite3",
                std::process::id()
            );
            Self {
                path: std::env::temp_dir().join(filename),
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TemporaryDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(sidecar_path(&self.path, "-shm"));
            let _ = fs::remove_file(sidecar_path(&self.path, "-wal"));
        }
    }

    #[test]
    fn mutations_commit_their_facts_and_data_revision_together() {
        let database = TemporaryDatabase::new("atomic-revision");
        let mut store = open_store(database.path(), 1);
        let missing_run_intent = active_intent(run_id(1), application_id(2), 0, 5);

        assert_eq!(
            store.writer().persist_tracking(&missing_run_intent),
            Err(StorageError::TrackerRunMissing)
        );
        assert_eq!(
            store
                .open_read_session()
                .and_then(|mut reader| reader.snapshot())
                .map(|snapshot| snapshot.revision().get()),
            Ok(0)
        );

        let writer = store.writer();
        assert_eq!(
            writer.register_tracker_run(&registration(1, 0)),
            Ok(revision(1))
        );
        seed_active_application(writer, 2);
        let committed = writer.persist_tracking(&active_intent(run_id(1), application_id(2), 0, 5));
        assert_eq!(committed, Ok(revision(4)));

        let snapshot = store
            .open_read_session()
            .and_then(|mut reader| reader.snapshot())
            .expect("the committed store should produce a snapshot");
        assert_eq!(snapshot.revision(), revision(4));
        assert!(snapshot.activities().is_empty());
        assert_eq!(snapshot.applications().len(), 1);
        assert_eq!(snapshot.categories().len(), 1);
    }

    #[test]
    fn schedule_persistence_writes_one_time_schedule_and_category_reference() {
        let database = TemporaryDatabase::new("schedule-one-time");
        let mut store = open_store(database.path(), 21);
        let category = Category::new(
            category_id(22),
            CategoryName::try_new("Planning").expect("valid category name"),
        );
        assert_eq!(
            store.writer().create_category(&category, UtcMicros::new(10)),
            Ok(revision(1))
        );
        let rule = ScheduleRule::one_time(
            "Doctor appointment",
            Some(category.id()),
            HalfOpenInterval::try_new(UtcMicros::new(100), UtcMicros::new(200))
                .expect("positive interval"),
            "America/Toronto",
        )
        .expect("valid one-time rule");
        let snapshot = ScheduleSnapshot::try_new(
            ScheduleId::OneTime(OneTimeScheduleId::from_bytes([23; 16])),
            rule,
            EntityRevision::new(0),
            UtcMicros::new(11),
        )
        .expect("matching one-time identity");

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &snapshot),
            Ok(revision(2))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &snapshot),
            Err(SchedulePersistenceError::Conflict)
        );
        let stored = store
            .writer()
            .writer
            .connection_mut()
            .query_row(
                "SELECT label, category_id IS NOT NULL, start_utc_us, end_utc_us,
                        created_zone_id, created_utc_us, updated_utc_us, revision
                   FROM one_time_schedule",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .expect("stored one-time schedule");
        assert_eq!(
            stored,
            (
                "Doctor appointment".to_owned(),
                true,
                100,
                200,
                "America/Toronto".to_owned(),
                11,
                11,
                0,
            )
        );
        assert_eq!(
            store
                .open_read_session()
                .and_then(|mut reader| reader.snapshot())
                .map(|read| {
                    assert_eq!(read.schedules().len(), 1);
                    assert_eq!(read.schedules()[0].snapshot(), &snapshot);
                    read.revision()
                }),
            Ok(revision(2))
        );
    }

    #[test]
    fn schedule_persistence_accepts_adjacency_and_rejects_one_time_overlap() {
        let database = TemporaryDatabase::new("schedule-one-time-overlap");
        let mut store = open_store(database.path(), 24);
        let first = one_time_schedule_snapshot(25, 100, 200);
        let adjacent = one_time_schedule_snapshot(26, 200, 300);
        let overlapping = one_time_schedule_snapshot(27, 150, 250);

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &first),
            Ok(revision(1))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &adjacent),
            Ok(revision(2))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &overlapping),
            Err(SchedulePersistenceError::Conflict)
        );
        assert_eq!(
            store
                .open_read_session()
                .and_then(|mut reader| reader.snapshot())
                .map(|snapshot| snapshot.revision()),
            Ok(revision(2))
        );
    }

    #[test]
    fn schedule_replacement_requires_the_current_revision_and_is_atomic() {
        let database = TemporaryDatabase::new("schedule-replacement");
        let mut store = open_store(database.path(), 26);
        let original = one_time_schedule_snapshot(28, 100, 200);
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &original),
            Ok(revision(1))
        );
        let replacement = ScheduleSnapshot::try_new(
            original.id(),
            ScheduleRule::one_time(
                "Moved appointment",
                None,
                HalfOpenInterval::try_new(UtcMicros::new(300), UtcMicros::new(400))
                    .expect("positive replacement interval"),
                "Etc/UTC",
            )
            .expect("valid replacement rule"),
            EntityRevision::new(1),
            UtcMicros::new(13),
        )
        .expect("matching schedule identity");
        assert_eq!(
            SchedulePersistence::replace_schedule(
                store.writer(),
                &replacement,
                EntityRevision::new(0),
            ),
            Ok(revision(2))
        );
        assert_eq!(
            SchedulePersistence::replace_schedule(
                store.writer(),
                &replacement,
                EntityRevision::new(0),
            ),
            Err(SchedulePersistenceError::RevisionConflict)
        );
    }

    #[test]
    fn schedule_deletion_requires_the_current_revision() {
        let database = TemporaryDatabase::new("schedule-deletion");
        let mut store = open_store(database.path(), 27);
        let schedule = one_time_schedule_snapshot(29, 100, 200);
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &schedule),
            Ok(revision(1))
        );
        assert_eq!(
            SchedulePersistence::delete_schedule(
                store.writer(),
                schedule.id(),
                EntityRevision::new(1),
            ),
            Err(SchedulePersistenceError::RevisionConflict)
        );
        assert_eq!(
            SchedulePersistence::delete_schedule(
                store.writer(),
                schedule.id(),
                EntityRevision::new(0),
            ),
            Ok(revision(2))
        );
    }

    #[test]
    fn schedule_persistence_rejects_recurring_conflict_with_one_time_schedule() {
        let database = TemporaryDatabase::new("schedule-recurring-one-time-overlap");
        let mut store = open_store(database.path(), 28);
        let one_time = one_time_schedule_snapshot(29, 32_700_000_000, 35_100_000_000);
        let recurring_rule = ScheduleRule::repeating(
            "Daily planning",
            None,
            0b0111_1111,
            9 * 3_600,
            10 * 3_600,
            0,
            None,
            "Etc/UTC",
        )
        .expect("valid recurring rule");
        let recurring = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([30; 16])),
            recurring_rule,
            EntityRevision::new(0),
            UtcMicros::new(12),
        )
        .expect("matching recurring identity");

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &one_time),
            Ok(revision(1))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &recurring),
            Err(SchedulePersistenceError::Conflict)
        );
        assert_eq!(
            store
                .open_read_session()
                .and_then(|mut reader| reader.snapshot())
                .map(|snapshot| snapshot.revision()),
            Ok(revision(1))
        );
    }

    #[test]
    fn schedule_persistence_rejects_recurring_conflict_with_recurring_schedule() {
        let database = TemporaryDatabase::new("schedule-recurring-recurring-overlap");
        let mut store = open_store(database.path(), 35);
        let first = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([35; 16])),
            ScheduleRule::repeating(
                "Daily planning",
                None,
                0b0111_1111,
                9 * 3_600,
                10 * 3_600,
                0,
                None,
                "Etc/UTC",
            )
            .expect("valid first recurring rule"),
            EntityRevision::new(0),
            UtcMicros::new(12),
        )
        .expect("matching first recurring identity");
        let second = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([36; 16])),
            ScheduleRule::repeating(
                "Overlapping review",
                None,
                0b0111_1111,
                9 * 3_600 + 30 * 60,
                10 * 3_600 + 30 * 60,
                0,
                None,
                "Etc/UTC",
            )
            .expect("valid second recurring rule"),
            EntityRevision::new(0),
            UtcMicros::new(12),
        )
        .expect("matching second recurring identity");

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &first),
            Ok(revision(1))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &second),
            Err(SchedulePersistenceError::Conflict)
        );
    }

    #[test]
    fn schedule_persistence_rejects_one_time_conflict_with_recurring_schedule() {
        let database = TemporaryDatabase::new("schedule-one-time-recurring-overlap");
        let mut store = open_store(database.path(), 30);
        let recurring = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([31; 16])),
            ScheduleRule::repeating(
                "Daily planning",
                None,
                0b0111_1111,
                9 * 3_600,
                10 * 3_600,
                0,
                None,
                "Etc/UTC",
            )
            .expect("valid recurring rule"),
            EntityRevision::new(0),
            UtcMicros::new(12),
        )
        .expect("matching recurring identity");
        let conflicting_one_time = one_time_schedule_snapshot(32, 32_700_000_000, 35_100_000_000);

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &recurring),
            Ok(revision(1))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &conflicting_one_time),
            Err(SchedulePersistenceError::Conflict)
        );
        assert_eq!(
            store
                .open_read_session()
                .and_then(|mut reader| reader.snapshot())
                .map(|snapshot| snapshot.revision()),
            Ok(revision(1))
        );
    }

    #[test]
    fn schedule_persistence_honors_persisted_recurring_skip_during_overlap_validation() {
        let database = TemporaryDatabase::new("schedule-recurring-skip-overlap");
        let mut store = open_store(database.path(), 31);
        let mut recurring_rule = ScheduleRule::repeating(
            "Daily planning",
            None,
            0b0111_1111,
            9 * 3_600,
            10 * 3_600,
            0,
            None,
            "Etc/UTC",
        )
        .expect("valid recurring rule");
        recurring_rule
            .skip_only_this_date(0)
            .expect("valid occurrence skip");
        let recurring = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([33; 16])),
            recurring_rule,
            EntityRevision::new(0),
            UtcMicros::new(12),
        )
        .expect("matching recurring identity");
        let one_time_at_skipped_occurrence =
            one_time_schedule_snapshot(34, 32_700_000_000, 35_100_000_000);

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &recurring),
            Ok(revision(1))
        );
        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &one_time_at_skipped_occurrence),
            Ok(revision(2))
        );
    }

    #[test]
    fn application_exclusion_policy_is_atomic_and_reversible() {
        let database = TemporaryDatabase::new("application-exclusion-policy");
        let mut store = open_store(database.path(), 31);
        let writer = store.writer();
        seed_active_application(writer, 32);
        assert_eq!(
            writer.set_applications_excluded(&[application_id(32)], true),
            Ok(revision(3))
        );
        let excluded: i64 = writer
            .writer
            .connection_mut()
            .query_row("SELECT exclusion_policy FROM application", [], |row| row.get(0))
            .expect("stored exclusion policy");
        assert_eq!(excluded, 1);
        assert_eq!(
            writer.set_applications_excluded(&[application_id(32)], false),
            Ok(revision(4))
        );
        let excluded: i64 = writer
            .writer
            .connection_mut()
            .query_row("SELECT exclusion_policy FROM application", [], |row| row.get(0))
            .expect("stored exclusion policy");
        assert_eq!(excluded, 0);
    }

    fn one_time_schedule_snapshot(
        id_byte: u8,
        start_utc_us: i64,
        end_utc_us: i64,
    ) -> ScheduleSnapshot {
        let rule = ScheduleRule::one_time(
            "Adjacent appointment",
            None,
            HalfOpenInterval::try_new(UtcMicros::new(start_utc_us), UtcMicros::new(end_utc_us))
                .expect("positive interval"),
            "Etc/UTC",
        )
        .expect("valid one-time rule");
        ScheduleSnapshot::try_new(
            ScheduleId::OneTime(OneTimeScheduleId::from_bytes([id_byte; 16])),
            rule,
            EntityRevision::new(0),
            UtcMicros::new(12),
        )
        .expect("matching one-time identity")
    }

    #[test]
    fn schedule_persistence_writes_segments_and_exception_provenance() {
        let database = TemporaryDatabase::new("schedule-series");
        let mut store = open_store(database.path(), 24);
        let mut rule = ScheduleRule::repeating(
            "Morning planning",
            None,
            0b0001_1111,
            9 * 3_600,
            10 * 3_600,
            20_000,
            None,
            "America/Toronto",
        )
        .expect("valid recurring rule");
        let override_interval = HalfOpenInterval::try_new(UtcMicros::new(300), UtcMicros::new(600))
            .expect("positive override");
        rule.override_only_this_date(20_005, override_interval, true, false, false, true)
            .expect("valid exception");
        let snapshot = ScheduleSnapshot::try_new(
            ScheduleId::Series(ScheduleSeriesId::from_bytes([25; 16])),
            rule,
            EntityRevision::new(4),
            UtcMicros::new(12),
        )
        .expect("matching recurring identity");

        assert_eq!(
            SchedulePersistence::create_schedule(store.writer(), &snapshot),
            Ok(revision(1))
        );
        let connection = store.writer().writer.connection_mut();
        let segment = stored_schedule_segment(connection);
        assert_eq!(
            segment,
            (
                31,
                32_400,
                36_000,
                0,
                "America/Toronto".to_owned(),
                "Morning planning".to_owned(),
                12,
                4,
            )
        );
        let exception = stored_schedule_exception(connection);
        assert_eq!(
            exception,
            (20_005, 1, 300, 600, "America/Toronto".to_owned(), 4, 1, 2)
        );
        let read = store
            .open_read_session()
            .and_then(|mut reader| reader.snapshot())
            .expect("correlated schedule read should succeed");
        assert_eq!(read.schedules().len(), 1);
        assert_eq!(read.schedules()[0].snapshot(), &snapshot);
    }

    fn stored_schedule_segment(
        connection: &mut Connection,
    ) -> (i64, i64, i64, i64, String, String, i64, i64) {
        connection
            .query_row(
                "SELECT weekday_mask, start_second_of_day, end_second_of_day, end_day_offset,
                        time_zone_id, label, created_utc_us, revision
                   FROM schedule_rule_segment",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .expect("stored schedule segment")
    }

    fn stored_schedule_exception(
        connection: &mut Connection,
    ) -> (i32, i64, i64, i64, String, i64, i64, i64) {
        connection
            .query_row(
                "SELECT anchor_local_date, kind, override_start_utc_us, override_end_utc_us,
                        resolved_zone_id, revision, start_boundary_resolution,
                        end_boundary_resolution
                   FROM schedule_exception",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .expect("stored schedule exception")
    }

    #[test]
    fn catalog_mutations_preserve_history_and_update_assignments_atomically() {
        let database = TemporaryDatabase::new("catalog-mutations");
        let mut store = open_store(database.path(), 12);
        let writer = store.writer();
        seed_active_application(writer, 1);
        seed_active_application(writer, 2);
        let category = Category::new(
            category_id(3),
            CategoryName::try_new("Work").expect("fixture category name should be valid"),
        );
        writer
            .create_category(&category, time(10))
            .expect("a new category should commit");
        writer
            .assign_applications(&[application_id(1), application_id(2)], Some(category.id()))
            .expect("the complete selected set should be assigned together");
        writer
            .rename_category(
                category.id(),
                &CategoryName::try_new("Deep work").expect("fixture category name should be valid"),
                time(20),
            )
            .expect("renaming an existing category should commit");

        assert_eq!(
            writer.assign_applications(&[application_id(1), application_id(9)], None),
            Err(StorageError::ApplicationMissing)
        );
        writer
            .delete_category(category.id())
            .expect("category deletion should return applications to Uncategorized");

        let snapshot = store
            .open_read_session()
            .and_then(|mut reader| reader.snapshot())
            .expect("the catalog should remain readable after all mutations");
        assert_eq!(snapshot.revision(), revision(8));
        assert!(
            snapshot
                .applications()
                .iter()
                .all(|record| record.application().category_id().is_none())
        );
        assert!(
            snapshot
                .categories()
                .iter()
                .all(|record| record.category().name().as_str() != "Deep work")
        );
    }

    #[test]
    fn focus_persistence_keeps_one_active_timer_and_reconciles_deadlines() {
        let database = TemporaryDatabase::new("focus-persistence");
        let mut store = open_store(database.path(), 13);
        let session_id = FocusSessionId::from_bytes([13; 16]);
        let draft = FocusSnapshot::try_new(
            session_id,
            FocusKind::Focus,
            Some("Finish report".to_owned()),
            50,
            None,
            EntityRevision::new(0),
        )
        .expect("fixture focus draft should be valid");
        {
            let mut service = FocusService::new(store.writer(), NoNotifications);
            assert!(matches!(
                service
                    .handle(&focus_command(1, FocusCommand::CreateDraft(draft)))
                    .outcome(),
                openmanic_application::MutationOutcome::Confirmed(_)
            ));
            assert!(matches!(
                service
                    .handle(&focus_command(
                        2,
                        FocusCommand::Start {
                            session_id,
                            started_at: time(100),
                        },
                    ))
                    .outcome(),
                openmanic_application::MutationOutcome::Confirmed(_)
            ));
            let restored = service
                .reconcile_after_restart(time(200))
                .expect("SQLite focus persistence should reconcile")
                .expect("running focus session should remain visible");
            assert!(matches!(
                restored.session().state(),
                FocusSessionState::Completed { completed_at, .. } if completed_at == time(150)
            ));
        }
        assert!(matches!(
            FocusPersistence::load_active_focus(store.writer()),
            Ok(None)
        ));
    }

    #[test]
    fn tracking_persistence_port_confirms_only_after_the_storage_commit() {
        let database = TemporaryDatabase::new("tracking-port");
        let mut store = open_store(database.path(), 8);
        let writer = store.writer();
        writer
            .register_tracker_run(&registration(8, 0))
            .expect("the tracker run should register");
        seed_active_application(writer, 9);

        let result = TrackingPersistencePort::try_persist(
            writer,
            active_intent(run_id(8), application_id(9), 0, 1),
        );
        assert!(matches!(result, TrackingPersistenceSubmit::Committed(_)));
        let snapshot = store
            .open_read_session()
            .and_then(|mut reader| reader.snapshot())
            .expect("a confirmed port result must have committed its checkpoint revision");
        assert_eq!(snapshot.revision().get(), 4);
    }

    #[test]
    fn closed_tracking_interval_and_replacement_checkpoint_share_one_revision() {
        let database = TemporaryDatabase::new("tracking-revision");
        let mut store = open_store(database.path(), 2);
        let writer = store.writer();
        writer
            .register_tracker_run(&registration(2, 0))
            .expect("the run should register");
        seed_active_application(writer, 3);
        writer
            .persist_tracking(&active_intent(run_id(2), application_id(3), 0, 10))
            .expect("the initial checkpoint should commit");

        let closed = active_interval(run_id(2), application_id(3), 0, 10);
        let checkpoint = checkpoint(
            run_id(2),
            10,
            20,
            ActivityState::UnknownMissing,
            ActivityCause::AdapterFailure,
            None,
        );
        let intent = TrackingPersistenceIntent::try_new(vec![closed], checkpoint)
            .expect("the adjacent transition should be a valid intent");
        let revision = writer
            .persist_tracking(&intent)
            .expect("the transition and checkpoint should commit together");

        let snapshot = store
            .open_read_session()
            .and_then(|mut reader| reader.snapshot())
            .expect("the committed transition should be visible");
        assert_eq!(snapshot.revision(), revision);
        assert_eq!(snapshot.activities().len(), 1);
        assert_eq!(snapshot.activities()[0].source_revision(), revision);
        assert_eq!(snapshot.activities()[0].interval(), closed);
    }

    #[test]
    fn busy_writer_fails_within_the_configured_bounded_wait() {
        let database = TemporaryDatabase::new("bounded-busy");
        let mut store = open_store(database.path(), 3);
        store
            .writer
            .writer
            .connection_mut()
            .busy_timeout(Duration::from_millis(1))
            .expect("the test writer timeout should be configurable");
        let blocker =
            Connection::open(database.path()).expect("a separate fixture connection should open");
        blocker
            .execute_batch("BEGIN IMMEDIATE")
            .expect("the fixture should hold the writer reservation");

        let category = Category::new(
            category_id(4),
            CategoryName::try_new("Busy fixture").expect("category name should be valid"),
        );
        assert_eq!(
            store.writer().upsert_category(&category, time(0)),
            Err(StorageError::Busy {
                operation: "begin category mutation",
            })
        );
        blocker
            .execute_batch("ROLLBACK")
            .expect("the fixture writer reservation should release");
    }

    #[test]
    fn writer_preserves_verified_wal_and_durability_configuration() {
        let database = TemporaryDatabase::new("wal-settings");
        let mut store = open_store(database.path(), 4);
        let configuration = store.writer().configuration();
        assert_eq!(configuration.journal_mode(), Some(JournalMode::Wal));
        assert_eq!(configuration.synchronous(), Some(SynchronousMode::Full));
        assert!(configuration.foreign_keys());
        assert!(!configuration.trusted_schema());
        assert!(!configuration.query_only());
        assert_eq!(configuration.busy_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn checkpoint_recovery_never_fabricates_activity_or_powered_off_time() {
        let database = TemporaryDatabase::new("checkpoint-recovery");
        let mut first_process = open_store(database.path(), 5);
        let writer = first_process.writer();
        writer
            .register_tracker_run(&registration(5, 0))
            .expect("the old tracker run should register");
        seed_active_application(writer, 6);
        writer
            .persist_tracking(&active_intent(run_id(5), application_id(6), 0, 10))
            .expect("the trusted checkpoint should commit before the simulated crash");
        drop(first_process);

        let mut recovered_process = open_store(database.path(), 5);
        let outcome = recovered_process
            .writer()
            .recover_unclean_exit(
                &active_intent(run_id(7), application_id(6), 30, 30),
                &registration(7, 30),
            )
            .expect("recovery should close only durable state and start a new run");
        assert!(matches!(
            outcome,
            RecoveryOutcome::Recovered {
                closed_through_checkpoint: true,
                recorded_unknown_gap: true,
                ..
            }
        ));
        let RecoveryOutcome::Recovered { revision, .. } = outcome else {
            return;
        };

        let snapshot = recovered_process
            .open_read_session()
            .and_then(|mut reader| reader.snapshot())
            .expect("the recovered store should remain readable");
        assert_eq!(snapshot.revision(), revision);
        assert_eq!(snapshot.activities().len(), 2);
        let trusted = snapshot.activities()[0].interval();
        assert_eq!(trusted.range(), range(0, 10));
        assert_eq!(trusted.state(), ActivityState::Active);
        assert_eq!(trusted.application_id(), Some(application_id(6)));
        let unknown = snapshot.activities()[1].interval();
        assert_eq!(unknown.range(), range(10, 30));
        assert_eq!(unknown.state(), ActivityState::UnknownMissing);
        assert_eq!(unknown.cause(), ActivityCause::CrashRecoveryGap);
        assert_eq!(unknown.application_id(), None);
        assert!(
            snapshot
                .activities()
                .iter()
                .all(|record| { record.interval().state() != ActivityState::PoweredOff })
        );
    }

    #[test]
    fn durable_enum_codes_match_the_immutable_initial_schema() {
        assert_eq!(activity_state_code(ActivityState::Active), 0);
        assert_eq!(activity_state_code(ActivityState::UnknownMissing), 6);
        assert_eq!(activity_cause_code(ActivityCause::ForegroundApplication), 0);
        assert_eq!(activity_cause_code(ActivityCause::ClockDiscontinuity), 14);
    }

    fn open_store(path: &Path, store_byte: u8) -> SqliteStore {
        SqliteStore::open(path, &StoreOpenOptions::new([store_byte; 16], 0, "test"))
            .expect("an isolated store should open")
    }

    fn seed_active_application(writer: &mut StorageWriter, byte: u8) {
        let category = Category::new(
            category_id(byte),
            CategoryName::try_new("Fixture category").expect("category name should be valid"),
        );
        writer
            .upsert_category(&category, time(0))
            .expect("the fixture category should commit");
        let application = Application::try_new(
            application_id(byte),
            ApplicationName::try_new("Fixture application")
                .expect("application name should be valid"),
            Some(category.id()),
            time(0),
            time(0),
        )
        .expect("fixture observation bounds should be valid");
        writer
            .upsert_application(&application)
            .expect("the fixture application should commit");
    }

    fn active_intent(
        tracker_run_id: TrackerRunId,
        application_id: ApplicationId,
        start_utc_us: i64,
        confirmed_utc_us: i64,
    ) -> TrackingPersistenceIntent {
        TrackingPersistenceIntent::try_new(
            Vec::new(),
            checkpoint(
                tracker_run_id,
                start_utc_us,
                confirmed_utc_us,
                ActivityState::Active,
                ActivityCause::ForegroundApplication,
                Some(application_id),
            ),
        )
        .expect("the fixture checkpoint should produce a valid intent")
    }

    fn active_interval(
        tracker_run_id: TrackerRunId,
        application_id: ApplicationId,
        start_utc_us: i64,
        end_utc_us: i64,
    ) -> ActivityInterval {
        ActivityInterval::try_new(
            tracker_run_id,
            range(start_utc_us, end_utc_us),
            ActivityState::Active,
            ActivityEvidence::try_from_cause(ActivityCause::ForegroundApplication)
                .expect("ordinary active evidence should be valid"),
            Some(application_id),
        )
        .expect("the fixture active interval should be valid")
    }

    fn checkpoint(
        tracker_run_id: TrackerRunId,
        start_utc_us: i64,
        confirmed_utc_us: i64,
        state: ActivityState,
        cause: ActivityCause,
        application_id: Option<ApplicationId>,
    ) -> TrackingCheckpoint {
        TrackingCheckpoint::try_new(
            tracker_run_id,
            time(start_utc_us),
            time(confirmed_utc_us),
            state,
            ActivityEvidence::try_from_cause(cause).expect("ordinary checkpoint cause should work"),
            application_id,
            1,
        )
        .expect("fixture checkpoint should satisfy state invariants")
    }

    fn registration(byte: u8, started_utc_us: i64) -> TrackerRunRegistration {
        TrackerRunRegistration::try_new(run_id(byte), time(started_utc_us), "test-adapter")
            .expect("fixture run registration should be valid")
    }

    fn run_id(byte: u8) -> TrackerRunId {
        TrackerRunId::from_bytes([byte; 16])
    }

    fn application_id(byte: u8) -> ApplicationId {
        ApplicationId::from_bytes([byte; 16])
    }

    fn category_id(byte: u8) -> CategoryId {
        CategoryId::from_bytes([byte; 16])
    }

    fn range(start_utc_us: i64, end_utc_us: i64) -> HalfOpenInterval {
        HalfOpenInterval::try_new(time(start_utc_us), time(end_utc_us))
            .expect("fixture range should be positive")
    }

    fn time(value: i64) -> UtcMicros {
        UtcMicros::new(value)
    }

    fn revision(value: u64) -> openmanic_application::DataRevision {
        openmanic_application::DataRevision::new(value)
    }

    struct NoNotifications;

    impl FocusNotificationPort for NoNotifications {
        fn notify_completed(&mut self, _: &FocusSnapshot) -> Result<(), FocusNotificationError> {
            Ok(())
        }
    }

    fn focus_command(id: u64, payload: FocusCommand) -> CommandEnvelope<FocusCommand> {
        CommandEnvelope::new(
            SchemaRevision::new(1),
            CommandId::new(id),
            OrderingKey::new(13),
            None,
            time(0),
            payload,
        )
    }

    fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
        let mut sidecar = OsString::from(path.as_os_str());
        sidecar.push(suffix);
        PathBuf::from(sidecar)
    }
}
