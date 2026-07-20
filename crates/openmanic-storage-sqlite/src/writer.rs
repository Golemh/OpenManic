//! Serialized authoritative writes and checkpoint recovery for the local store.

use std::{
    fs,
    path::{Path, PathBuf},
};

use openmanic_application::{
    AcceptedWindowTitle, ApplicationError, ApplicationPort, CancellationToken, CatalogPersistence,
    CatalogPersistenceError, CsvImportRequest, DataRevision, EntityRevision, FocusKind,
    FocusPersistence, FocusPersistenceError, FocusSnapshot, ImportFailure, ImportScopeOutcome,
    LayoutPersistence, LayoutPersistenceError, LayoutSnapshot, PortFailureReason, SavedViewId,
    SavedViewPersistence, SavedViewPersistenceError, SavedViewSnapshot, ScheduleId,
    SchedulePersistence, SchedulePersistenceError, ScheduleSnapshot, SettingsPersistence,
    SettingsPersistenceError, SettingsSnapshot, SettingsThemeMode, TrackingPersistenceIntent,
    TrackingPersistencePort, TrackingPersistenceSubmit, repeating_schedule_rules_conflict,
    schedule_rule_conflicts_with_intervals,
};
use openmanic_domain::{
    ActivityCause, ActivityInterval, ActivityState, Application, ApplicationId, Category,
    CategoryId, CategoryName, FocusSessionId, FocusSessionState, HalfOpenInterval, LayoutDocument,
    ScheduleOccurrenceException, ScheduleSegment, TrackerRunId, UtcMicros,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::backup::{create_user_backup, restore_user_backup};
use crate::repository::{
    database_error, decode_layout_document, encode_layout_definition, encode_saved_view_definition,
    load_saved_views, read_schedule_snapshot,
};
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

type StoredSettingsRow = (
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
);

impl StorageWriter {
    /// Loads the complete authoritative settings singleton when it exists.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] for malformed persisted settings values or read failures.
    pub fn settings_snapshot(&mut self) -> Result<Option<SettingsSnapshot>, StorageError> {
        let row: Option<StoredSettingsRow> = self.writer.connection_mut().query_row(
            "SELECT first_launch_consent_revision, start_tracking_automatically, start_at_login, close_to_tray, idle_threshold_seconds, idle_policy, collect_window_titles, time_zone_mode, manual_time_zone_id, theme_mode, density, notifications_enabled, focus_sounds_enabled, tray_explanation_acknowledged, revision FROM user_settings WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?)),
        ).optional().map_err(|error| database_error(&error, "read settings snapshot"))?;
        row.map(
            |(
                consent,
                tracking,
                login,
                close,
                idle_seconds,
                idle_policy,
                titles,
                time_zone,
                manual_zone,
                theme,
                density,
                notifications,
                sounds,
                tray_acknowledged,
                revision,
            )| {
                Ok(SettingsSnapshot::new(
                    u32::try_from(consent).map_err(|_| StorageError::InvalidStoredValue {
                        field: "settings consent revision",
                    })?,
                    setting_bool(tracking, "settings tracking start")?,
                    setting_bool(login, "settings login start")?,
                    setting_bool(close, "settings close behavior")?,
                    u32::try_from(idle_seconds).map_err(|_| StorageError::InvalidStoredValue {
                        field: "settings idle threshold",
                    })?,
                    u16::try_from(idle_policy).map_err(|_| StorageError::InvalidStoredValue {
                        field: "settings idle policy",
                    })?,
                    setting_bool(titles, "settings title collection")?,
                    u8::try_from(time_zone).map_err(|_| StorageError::InvalidStoredValue {
                        field: "settings time zone mode",
                    })?,
                    manual_zone,
                    SettingsThemeMode::try_from_code(u8::try_from(theme).map_err(|_| {
                        StorageError::InvalidStoredValue {
                            field: "settings theme mode",
                        }
                    })?)
                    .map_err(|_| StorageError::InvalidStoredValue {
                        field: "settings theme mode",
                    })?,
                    u16::try_from(density).map_err(|_| StorageError::InvalidStoredValue {
                        field: "settings density",
                    })?,
                    setting_bool(notifications, "settings notifications")?,
                    setting_bool(sounds, "settings focus sounds")?,
                    setting_bool(tray_acknowledged, "settings tray acknowledgement")?,
                    EntityRevision::new(u64::try_from(revision).map_err(|_| {
                        StorageError::InvalidStoredValue {
                            field: "settings revision",
                        }
                    })?),
                ))
            },
        )
        .transpose()
    }
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

    /// Persists one accepted stabilized title span, deduplicating text and coalescing an adjacent
    /// identical span for the same application and tracker run.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the referenced application or tracker run is absent, or the
    /// serialized transaction cannot commit the title fact and matching data revision.
    pub fn persist_window_title(
        &mut self,
        tracker_run_id: TrackerRunId,
        title: &AcceptedWindowTitle,
    ) -> Result<DataRevision, StorageError> {
        let transaction = self.begin_writer_transaction("begin window title persistence")?;
        let revision = next_revision(&transaction)?;
        let application_row_id =
            application_row_id(&transaction, title.application_id().as_bytes())?;
        let tracker_run_row_id = tracker_run_row_id(&transaction, tracker_run_id)?;
        transaction.execute(
            "INSERT INTO window_title_text(text_hash, title) VALUES (?1, ?2) ON CONFLICT(text_hash, title) DO NOTHING",
            params![title.text_hash().to_be_bytes().as_slice(), title.text()],
        ).map_err(|error| database_error(&error, "insert window title text"))?;
        let title_text_id: i64 = transaction
            .query_row(
                "SELECT id FROM window_title_text WHERE text_hash = ?1 AND title = ?2",
                params![title.text_hash().to_be_bytes().as_slice(), title.text()],
                |row| row.get(0),
            )
            .map_err(|error| database_error(&error, "find window title text"))?;
        let start = title.stable_since_utc().get();
        let end = title.accepted_at_utc().get();
        let revision_value =
            i64::try_from(revision.get()).map_err(|_| StorageError::InvalidStoredValue {
                field: "data revision",
            })?;
        let updated = transaction.execute(
            "UPDATE window_title_span SET end_utc_us = ?1, source_revision = ?2
              WHERE id = (SELECT id FROM window_title_span
                           WHERE application_id = ?3 AND tracker_run_id = ?4 AND title_text_id = ?5 AND end_utc_us = ?6
                           ORDER BY id DESC LIMIT 1)",
            params![end, revision_value, application_row_id, tracker_run_row_id, title_text_id, start],
        ).map_err(|error| database_error(&error, "coalesce window title span"))?;
        if updated == 0 {
            transaction.execute(
                "INSERT INTO window_title_span(application_id, tracker_run_id, title_text_id, start_utc_us, end_utc_us, source_revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![application_row_id, tracker_run_row_id, title_text_id, start, end, revision_value],
            ).map_err(|error| database_error(&error, "insert window title span"))?;
        }
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit window title persistence"))?;
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

    /// Returns whether the sole persisted settings record has explicitly enabled title collection.
    ///
    /// A missing record is deliberately treated as disabled: collection cannot begin before the
    /// user has an authoritative privacy setting.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the settings record cannot be read or contains an invalid
    /// persisted boolean value.
    pub fn window_title_collection_enabled(&mut self) -> Result<bool, StorageError> {
        Ok(self
            .settings_snapshot()?
            .is_some_and(|settings| settings.collect_window_titles()))
    }

    /// Persists one approved built-in theme mode through the singleton settings record.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the mode is unsupported or the atomic settings update fails.
    pub fn set_theme_mode(
        &mut self,
        theme_mode: u8,
        updated_at_utc: UtcMicros,
    ) -> Result<DataRevision, StorageError> {
        if theme_mode > 2 {
            return Err(StorageError::InvalidStoredValue {
                field: "theme mode",
            });
        }
        let transaction = self.begin_writer_transaction("begin theme settings update")?;
        let revision = next_revision(&transaction)?;
        transaction
            .execute(
                "INSERT INTO user_settings(
                singleton_id, schema_version, first_launch_consent_revision,
                start_tracking_automatically, start_at_login, close_to_tray,
                idle_threshold_seconds, idle_policy, collect_window_titles,
                time_zone_mode, manual_time_zone_id, theme_mode, density,
                notifications_enabled, focus_sounds_enabled, tray_explanation_acknowledged,
                revision, updated_utc_us
             ) VALUES (1, 1, 0, 1, 0, 1, 300, 1, 0, 0, NULL, ?1, 1, 1, 1, 0, 0, ?2)
             ON CONFLICT(singleton_id) DO UPDATE SET theme_mode = excluded.theme_mode,
                 revision = user_settings.revision + 1, updated_utc_us = excluded.updated_utc_us",
                params![i64::from(theme_mode), updated_at_utc.get()],
            )
            .map_err(|error| database_error(&error, "persist theme mode"))?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit theme settings update"))?;
        Ok(revision)
    }

    /// Returns the approved built-in theme mode, or no value before settings are first saved.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the stored mode is outside the supported built-in range.
    pub fn theme_mode(&mut self) -> Result<Option<u8>, StorageError> {
        Ok(self
            .settings_snapshot()?
            .map(|settings| settings.theme_mode().code()))
    }

    /// Imports the OpenManic CSV v1 interchange into the current local store.
    ///
    /// The source is parsed into a connection-local staging table before durable merge work
    /// starts. Malformed rows are retained as safe, row-numbered failures on the import batch;
    /// their original field values are never persisted in error text. Reimporting a file exported
    /// by this store is deterministic: categories and applications upsert by public ID and exact
    /// activity tuples are not inserted twice.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the local source cannot be read or SQLite cannot persist the
    /// batch, job, failures, or accepted records.
    pub fn import_csv(
        &mut self,
        request: &CsvImportRequest,
        completed_at_utc: UtcMicros,
    ) -> Result<ImportScopeOutcome, StorageError> {
        let (_source, cancellation) = openmanic_application::CancellationSource::new();
        self.import_csv_cancellable(request, completed_at_utc, &cancellation)?
            .ok_or(StorageError::DataOperationFailed {
                operation: "cancel CSV import",
            })
    }

    /// Imports CSV data while honoring cancellation before and during the transactional merge.
    ///
    /// A cancellation leaves the import batch and job records in their terminal cancelled state,
    /// retains row validation failures already discovered, and rolls back every merged entity.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the local source cannot be read or SQLite cannot persist the
    /// import batch, staged rows, cancellation checkpoint, or completed result.
    #[expect(
        clippy::too_many_lines,
        reason = "one transaction deliberately owns the complete CSV import lifecycle"
    )]
    pub fn import_csv_cancellable(
        &mut self,
        request: &CsvImportRequest,
        completed_at_utc: UtcMicros,
        cancellation: &CancellationToken,
    ) -> Result<Option<ImportScopeOutcome>, StorageError> {
        let bytes =
            fs::read(request.source().path()).map_err(|_| StorageError::DataOperationFailed {
                operation: "read CSV import",
            })?;
        let records = parse_csv_records(&bytes)?;
        let fingerprint = csv_fingerprint(&bytes);
        let transaction = self.begin_writer_transaction("begin CSV import")?;
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS import_stage_v1(
                 source_line INTEGER NOT NULL, record_type TEXT NOT NULL, stable_id TEXT NOT NULL,
                 start_utc_us TEXT NOT NULL, end_utc_us TEXT NOT NULL, activity_state TEXT NOT NULL,
                 activity_cause TEXT NOT NULL, application_id TEXT NOT NULL, category_id TEXT NOT NULL,
                 display_name TEXT NOT NULL
             ) STRICT; DELETE FROM import_stage_v1;",
        ).map_err(|error| database_error(&error, "prepare CSV import staging"))?;
        transaction.execute(
            "INSERT INTO job_record(public_id, kind, state, progress_current, progress_total,
                 source_reference, destination_reference, safe_checkpoint, error_summary,
                 created_utc_us, started_utc_us, completed_utc_us)
             VALUES (?1, 6, 1, 0, NULL, 'local-csv', 'current-store', NULL, NULL, ?2, ?2, NULL)
             ON CONFLICT(public_id) DO UPDATE SET state = 1, progress_current = 0,
                 progress_total = NULL, error_summary = NULL, started_utc_us = excluded.started_utc_us,
                 completed_utc_us = NULL",
            params![job_public_id(request.job_id().get()).as_slice(), completed_at_utc.get()],
        ).map_err(|error| database_error(&error, "create CSV import job"))?;
        transaction.execute(
            "INSERT INTO import_batch(public_id, file_fingerprint, format_schema_version, state,
                 parsed_count, accepted_count, rejected_count, committed_count, created_utc_us,
                 completed_utc_us, error_report_reference)
             VALUES (?1, ?2, 1, 1, 0, 0, 0, 0, ?3, NULL, NULL)
             ON CONFLICT(public_id) DO UPDATE SET file_fingerprint = excluded.file_fingerprint,
                 format_schema_version = 1, state = 1, parsed_count = 0, accepted_count = 0,
                 rejected_count = 0, committed_count = 0, created_utc_us = excluded.created_utc_us,
                 completed_utc_us = NULL, error_report_reference = NULL",
            params![request.batch_id().as_bytes().as_slice(), fingerprint.as_slice(), completed_at_utc.get()],
        ).map_err(|error| database_error(&error, "create CSV import batch"))?;
        let batch_row_id: i64 = transaction
            .query_row(
                "SELECT id FROM import_batch WHERE public_id = ?1",
                [request.batch_id().as_bytes().as_slice()],
                |row| row.get(0),
            )
            .map_err(|error| database_error(&error, "find CSV import batch"))?;
        transaction
            .execute(
                "DELETE FROM import_error WHERE import_batch_id = ?1",
                [batch_row_id],
            )
            .map_err(|error| database_error(&error, "clear CSV import errors"))?;

        let mut parsed = 0_u64;
        let mut accepted = 0_u64;
        for record in records.into_iter().skip(1) {
            if cancellation.is_cancelled() {
                complete_cancelled_csv_import(
                    &transaction,
                    request,
                    batch_row_id,
                    parsed,
                    accepted,
                    completed_at_utc,
                    "staged",
                )?;
                transaction
                    .commit()
                    .map_err(|error| database_error(&error, "commit cancelled CSV import"))?;
                return Ok(None);
            }
            parsed = parsed.saturating_add(1);
            match validate_csv_record(&record) {
                Ok(fields) => {
                    transaction.execute(
                        "INSERT INTO import_stage_v1 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![record.line, fields[0], fields[2], fields[5], fields[6], fields[7], fields[8], fields[9], fields[10], fields[11]],
                    ).map_err(|error| database_error(&error, "stage CSV import row"))?;
                    accepted = accepted.saturating_add(1);
                }
                Err((field, code)) => {
                    let failure = ImportFailure::try_new(
                        record.line.cast_unsigned(),
                        field.map(str::to_owned),
                        code,
                    )
                    .map_err(|_| StorageError::DataOperationFailed {
                        operation: "create CSV import failure",
                    })?;
                    insert_import_failure(&transaction, batch_row_id, &failure)?;
                }
            }
        }
        if cancellation.is_cancelled() {
            complete_cancelled_csv_import(
                &transaction,
                request,
                batch_row_id,
                parsed,
                accepted,
                completed_at_utc,
                "staged",
            )?;
            transaction
                .commit()
                .map_err(|error| database_error(&error, "commit cancelled CSV import"))?;
            return Ok(None);
        }
        let revision = next_revision(&transaction)?;
        transaction
            .execute_batch("SAVEPOINT csv_import_merge")
            .map_err(|error| database_error(&error, "begin CSV import merge savepoint"))?;
        if merge_csv_stage(&transaction, revision, batch_row_id, cancellation)? {
            transaction
                .execute_batch("ROLLBACK TO csv_import_merge; RELEASE csv_import_merge")
                .map_err(|error| database_error(&error, "rollback cancelled CSV import merge"))?;
            complete_cancelled_csv_import(
                &transaction,
                request,
                batch_row_id,
                parsed,
                accepted,
                completed_at_utc,
                "staged",
            )?;
            transaction
                .commit()
                .map_err(|error| database_error(&error, "commit cancelled CSV import"))?;
            return Ok(None);
        }
        transaction
            .execute_batch("RELEASE csv_import_merge")
            .map_err(|error| database_error(&error, "release CSV import merge savepoint"))?;
        let rejected: u64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM import_error WHERE import_batch_id = ?1",
                [batch_row_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| database_error(&error, "count CSV import failures"))?
            .try_into()
            .map_err(|_| StorageError::InvalidStoredValue {
                field: "CSV rejected count",
            })?;
        accepted = parsed.saturating_sub(rejected);
        // An accepted row has either been upserted, inserted, or was already present as the
        // exact idempotent tuple. Every rejected row is retained in `import_error` above.
        let committed = accepted;
        transaction
            .execute(
                "UPDATE import_batch SET state = 2, parsed_count = ?2, accepted_count = ?3,
                 rejected_count = ?4, committed_count = ?5, completed_utc_us = ?6 WHERE id = ?1",
                params![
                    batch_row_id,
                    as_i64(parsed, "CSV parsed count")?,
                    as_i64(accepted, "CSV accepted count")?,
                    as_i64(rejected, "CSV rejected count")?,
                    as_i64(committed, "CSV committed count")?,
                    completed_at_utc.get()
                ],
            )
            .map_err(|error| database_error(&error, "complete CSV import batch"))?;
        transaction
            .execute(
                "UPDATE job_record SET state = 2, progress_current = ?2, progress_total = ?2,
                 safe_checkpoint = 'merged', completed_utc_us = ?3 WHERE public_id = ?1",
                params![
                    job_public_id(request.job_id().get()).as_slice(),
                    as_i64(parsed, "CSV progress")?,
                    completed_at_utc.get()
                ],
            )
            .map_err(|error| database_error(&error, "complete CSV import job"))?;
        update_revision(&transaction, revision)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit CSV import"))?;
        ImportScopeOutcome::try_new(parsed, accepted, rejected, committed)
            .map(Some)
            .map_err(|_| StorageError::InvalidStoredValue {
                field: "CSV import counts",
            })
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

impl SavedViewPersistence for StorageWriter {
    fn load_saved_views(
        &mut self,
    ) -> Result<openmanic_application::SavedViewLoad, SavedViewPersistenceError> {
        load_saved_views(self.writer.connection_mut())
            .map_err(|_| SavedViewPersistenceError::Failed)
    }

    fn create_saved_view(
        &mut self,
        snapshot: &SavedViewSnapshot,
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin saved-view creation")
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        let revision =
            next_revision(&transaction).map_err(|_| SavedViewPersistenceError::Failed)?;
        insert_saved_view(&transaction, snapshot)
            .map_err(|_| SavedViewPersistenceError::InvalidDocument)?;
        update_revision(&transaction, revision).map_err(|_| SavedViewPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        Ok(revision)
    }

    fn replace_saved_view(
        &mut self,
        snapshot: &SavedViewSnapshot,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin saved-view replacement")
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        let revision =
            next_revision(&transaction).map_err(|_| SavedViewPersistenceError::Failed)?;
        let next_entity_revision = expected_revision
            .get()
            .checked_add(1)
            .ok_or(SavedViewPersistenceError::Failed)?;
        let changed = replace_saved_view(
            &transaction,
            snapshot,
            expected_revision,
            next_entity_revision,
        )
        .map_err(|_| SavedViewPersistenceError::InvalidDocument)?;
        if changed != 1 {
            return Err(SavedViewPersistenceError::RevisionConflict);
        }
        update_revision(&transaction, revision).map_err(|_| SavedViewPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        Ok(revision)
    }

    fn reorder_saved_views(
        &mut self,
        ordered: &[(SavedViewId, EntityRevision)],
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin saved-view reorder")
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        let total: i64 = transaction
            .query_row("SELECT COUNT(*) FROM saved_overview_view", [], |row| {
                row.get(0)
            })
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        if usize::try_from(total).ok() != Some(ordered.len()) {
            return Err(SavedViewPersistenceError::RevisionConflict);
        }
        for (id, expected_revision) in ordered {
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM saved_overview_view WHERE public_id = ?1",
                    [id.as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| SavedViewPersistenceError::Failed)?;
            if current
                .and_then(|value| u64::try_from(value).ok())
                .map(EntityRevision::new)
                != Some(*expected_revision)
            {
                return Err(SavedViewPersistenceError::RevisionConflict);
            }
        }
        let revision =
            next_revision(&transaction).map_err(|_| SavedViewPersistenceError::Failed)?;
        for (display_order, (id, expected_revision)) in ordered.iter().enumerate() {
            let display_order =
                i64::try_from(display_order).map_err(|_| SavedViewPersistenceError::Failed)?;
            let next_entity_revision = expected_revision
                .get()
                .checked_add(1)
                .ok_or(SavedViewPersistenceError::Failed)?;
            let changed = transaction.execute(
                "UPDATE saved_overview_view SET display_order = ?1, revision = ?2 WHERE public_id = ?3 AND revision = ?4",
                params![display_order, i64::try_from(next_entity_revision).map_err(|_| SavedViewPersistenceError::Failed)?, id.as_bytes().as_slice(), i64::try_from(expected_revision.get()).map_err(|_| SavedViewPersistenceError::Failed)?],
            ).map_err(|_| SavedViewPersistenceError::Failed)?;
            if changed != 1 {
                return Err(SavedViewPersistenceError::RevisionConflict);
            }
        }
        update_revision(&transaction, revision).map_err(|_| SavedViewPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        Ok(revision)
    }

    fn delete_saved_view(
        &mut self,
        id: SavedViewId,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin saved-view deletion")
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        let revision =
            next_revision(&transaction).map_err(|_| SavedViewPersistenceError::Failed)?;
        let changed = transaction
            .execute(
                "DELETE FROM saved_overview_view WHERE public_id = ?1 AND revision = ?2",
                params![
                    id.as_bytes().as_slice(),
                    i64::try_from(expected_revision.get())
                        .map_err(|_| SavedViewPersistenceError::Failed)?
                ],
            )
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        if changed != 1 {
            return Err(SavedViewPersistenceError::RevisionConflict);
        }
        update_revision(&transaction, revision).map_err(|_| SavedViewPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SavedViewPersistenceError::Failed)?;
        Ok(revision)
    }
}

impl SavedViewPersistence for &mut StorageWriter {
    fn load_saved_views(
        &mut self,
    ) -> Result<openmanic_application::SavedViewLoad, SavedViewPersistenceError> {
        <StorageWriter as SavedViewPersistence>::load_saved_views(*self)
    }
    fn create_saved_view(
        &mut self,
        snapshot: &SavedViewSnapshot,
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        <StorageWriter as SavedViewPersistence>::create_saved_view(*self, snapshot)
    }
    fn replace_saved_view(
        &mut self,
        snapshot: &SavedViewSnapshot,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        <StorageWriter as SavedViewPersistence>::replace_saved_view(
            *self,
            snapshot,
            expected_revision,
        )
    }
    fn reorder_saved_views(
        &mut self,
        ordered: &[(SavedViewId, EntityRevision)],
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        <StorageWriter as SavedViewPersistence>::reorder_saved_views(*self, ordered)
    }
    fn delete_saved_view(
        &mut self,
        id: SavedViewId,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SavedViewPersistenceError> {
        <StorageWriter as SavedViewPersistence>::delete_saved_view(*self, id, expected_revision)
    }
}

impl LayoutPersistence for StorageWriter {
    fn load_layout(&mut self) -> Result<Option<LayoutSnapshot>, LayoutPersistenceError> {
        let row: Option<(i64, i64, String, i64)> = self
            .writer
            .connection_mut()
            .query_row(
                "SELECT schema_version, revision, document_json, updated_utc_us FROM dashboard_layout WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| LayoutPersistenceError::Failed)?;
        row.map(|(schema_version, revision, source, updated_at_utc)| {
            let schema_version = u16::try_from(schema_version)
                .map_err(|_| LayoutPersistenceError::InvalidDocument)?;
            let revision =
                u64::try_from(revision).map_err(|_| LayoutPersistenceError::InvalidDocument)?;
            let document = decode_layout_document(&source, revision)
                .map_err(|_| LayoutPersistenceError::InvalidDocument)?;
            if document.schema_version() != schema_version {
                return Err(LayoutPersistenceError::InvalidDocument);
            }
            Ok(LayoutSnapshot::new(
                document,
                EntityRevision::new(revision),
                UtcMicros::new(updated_at_utc),
            ))
        })
        .transpose()
    }

    fn replace_layout(
        &mut self,
        snapshot: &LayoutSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> Result<DataRevision, LayoutPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin dashboard layout replacement")
            .map_err(|_| LayoutPersistenceError::Failed)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM dashboard_layout WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| LayoutPersistenceError::Failed)?;
        let existing = existing
            .map(|revision| {
                u64::try_from(revision)
                    .map(EntityRevision::new)
                    .map_err(|_| LayoutPersistenceError::InvalidDocument)
            })
            .transpose()?;
        if existing != expected_revision {
            return Err(LayoutPersistenceError::RevisionConflict);
        }
        let entity_revision = expected_revision
            .map_or(0, EntityRevision::get)
            .checked_add(u64::from(existing.is_some()))
            .ok_or(LayoutPersistenceError::Failed)?;
        let document = LayoutDocument::try_new(snapshot.document().definition(), entity_revision)
            .map_err(|_| LayoutPersistenceError::InvalidDocument)?;
        let revision = next_revision(&transaction).map_err(|_| LayoutPersistenceError::Failed)?;
        if existing.is_some() {
            transaction
                .execute(
                    "UPDATE dashboard_layout SET schema_version = ?1, revision = ?2, document_json = ?3, updated_utc_us = ?4 WHERE id = 1",
                    params![
                        i64::from(document.schema_version()),
                        i64::try_from(entity_revision).map_err(|_| LayoutPersistenceError::Failed)?,
                        encode_layout_definition(&document.definition()),
                        snapshot.updated_at_utc().get(),
                    ],
                )
                .map_err(|_| LayoutPersistenceError::Failed)?;
        } else {
            transaction
                .execute(
                    "INSERT INTO dashboard_layout(id, schema_version, revision, document_json, updated_utc_us) VALUES (1, ?1, ?2, ?3, ?4)",
                    params![
                        i64::from(document.schema_version()),
                        i64::try_from(entity_revision).map_err(|_| LayoutPersistenceError::Failed)?,
                        encode_layout_definition(&document.definition()),
                        snapshot.updated_at_utc().get(),
                    ],
                )
                .map_err(|_| LayoutPersistenceError::Failed)?;
        }
        update_revision(&transaction, revision).map_err(|_| LayoutPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| LayoutPersistenceError::Failed)?;
        Ok(revision)
    }
}

impl LayoutPersistence for &mut StorageWriter {
    fn load_layout(&mut self) -> Result<Option<LayoutSnapshot>, LayoutPersistenceError> {
        <StorageWriter as LayoutPersistence>::load_layout(*self)
    }

    fn replace_layout(
        &mut self,
        snapshot: &LayoutSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> Result<DataRevision, LayoutPersistenceError> {
        <StorageWriter as LayoutPersistence>::replace_layout(*self, snapshot, expected_revision)
    }
}

impl SettingsPersistence for StorageWriter {
    fn load_settings(&mut self) -> Result<Option<SettingsSnapshot>, SettingsPersistenceError> {
        self.settings_snapshot()
            .map_err(|error| settings_persistence_error(&error))
    }

    fn replace_settings(
        &mut self,
        settings: &SettingsSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> Result<DataRevision, SettingsPersistenceError> {
        let transaction = self
            .begin_writer_transaction("begin settings replacement")
            .map_err(|_| SettingsPersistenceError::Failed)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM user_settings WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| SettingsPersistenceError::Failed)?;
        let existing = existing
            .map(|revision| {
                u64::try_from(revision)
                    .map(EntityRevision::new)
                    .map_err(|_| SettingsPersistenceError::InvalidDocument)
            })
            .transpose()?;
        if existing != expected_revision {
            return Err(SettingsPersistenceError::RevisionConflict);
        }
        let entity_revision = expected_revision
            .map_or(0, EntityRevision::get)
            .checked_add(u64::from(existing.is_some()))
            .ok_or(SettingsPersistenceError::Failed)?;
        let revision = next_revision(&transaction).map_err(|_| SettingsPersistenceError::Failed)?;
        let parameters = params![
            i64::from(settings.consent_revision()),
            i64::from(u8::from(settings.start_tracking_automatically())),
            i64::from(u8::from(settings.start_at_login())),
            i64::from(u8::from(settings.close_to_tray())),
            i64::from(settings.idle_threshold_seconds()),
            i64::from(settings.idle_policy_code()),
            i64::from(u8::from(settings.collect_window_titles())),
            i64::from(settings.time_zone_mode()),
            settings.manual_time_zone_id(),
            i64::from(settings.theme_mode().code()),
            i64::from(settings.density_code()),
            i64::from(u8::from(settings.notifications_enabled())),
            i64::from(u8::from(settings.focus_sounds_enabled())),
            i64::from(u8::from(settings.tray_explanation_acknowledged())),
            i64::try_from(entity_revision).map_err(|_| SettingsPersistenceError::Failed)?,
        ];
        if existing.is_some() {
            transaction
                .execute(
                    "UPDATE user_settings SET first_launch_consent_revision = ?1, start_tracking_automatically = ?2, start_at_login = ?3, close_to_tray = ?4, idle_threshold_seconds = ?5, idle_policy = ?6, collect_window_titles = ?7, time_zone_mode = ?8, manual_time_zone_id = ?9, theme_mode = ?10, density = ?11, notifications_enabled = ?12, focus_sounds_enabled = ?13, tray_explanation_acknowledged = ?14, revision = ?15 WHERE singleton_id = 1",
                    parameters,
                )
                .map_err(|_| SettingsPersistenceError::Failed)?;
        } else {
            transaction
                .execute(
                    "INSERT INTO user_settings(singleton_id, schema_version, first_launch_consent_revision, start_tracking_automatically, start_at_login, close_to_tray, idle_threshold_seconds, idle_policy, collect_window_titles, time_zone_mode, manual_time_zone_id, theme_mode, density, notifications_enabled, focus_sounds_enabled, tray_explanation_acknowledged, revision, updated_utc_us) VALUES (1, 1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0)",
                    parameters,
                )
                .map_err(|_| SettingsPersistenceError::Failed)?;
        }
        update_revision(&transaction, revision).map_err(|_| SettingsPersistenceError::Failed)?;
        transaction
            .commit()
            .map_err(|_| SettingsPersistenceError::Failed)?;
        Ok(revision)
    }
}

impl SettingsPersistence for &mut StorageWriter {
    fn load_settings(&mut self) -> Result<Option<SettingsSnapshot>, SettingsPersistenceError> {
        <StorageWriter as SettingsPersistence>::load_settings(*self)
    }

    fn replace_settings(
        &mut self,
        settings: &SettingsSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> Result<DataRevision, SettingsPersistenceError> {
        <StorageWriter as SettingsPersistence>::replace_settings(*self, settings, expected_revision)
    }
}

fn settings_persistence_error(error: &StorageError) -> SettingsPersistenceError {
    match error {
        StorageError::InvalidStoredValue { .. } => SettingsPersistenceError::InvalidDocument,
        _ => SettingsPersistenceError::Failed,
    }
}

fn insert_saved_view(
    transaction: &Transaction<'_>,
    snapshot: &SavedViewSnapshot,
) -> Result<(), StorageError> {
    let document = snapshot.document();
    transaction.execute(
        "INSERT INTO saved_overview_view(public_id, name, display_order, schema_version, revision, definition_json, created_utc_us, updated_utc_us) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![snapshot.id().as_bytes().as_slice(), document.name(), i64::from(document.display_order()), i64::from(document.schema_version()), i64::try_from(snapshot.entity_revision().get()).map_err(|_| StorageError::InvalidStoredValue { field: "saved-view revision" })?, encode_saved_view_definition(&document.definition()), snapshot.created_at_utc().get()],
    ).map_err(|error| database_error(&error, "create saved view"))?;
    Ok(())
}

fn replace_saved_view(
    transaction: &Transaction<'_>,
    snapshot: &SavedViewSnapshot,
    expected_revision: EntityRevision,
    next_entity_revision: u64,
) -> Result<usize, StorageError> {
    let definition = snapshot.document().definition();
    let document = openmanic_domain::SavedViewDocument::try_new(definition, next_entity_revision)
        .map_err(|_| StorageError::InvalidStoredValue {
        field: "saved-view document",
    })?;
    transaction.execute(
        "UPDATE saved_overview_view SET name = ?1, display_order = ?2, schema_version = ?3, revision = ?4, definition_json = ?5, updated_utc_us = ?6 WHERE public_id = ?7 AND revision = ?8",
        params![document.name(), i64::from(document.display_order()), i64::from(document.schema_version()), i64::try_from(next_entity_revision).map_err(|_| StorageError::InvalidStoredValue { field: "saved-view revision" })?, encode_saved_view_definition(&document.definition()), snapshot.created_at_utc().get(), snapshot.id().as_bytes().as_slice(), i64::try_from(expected_revision.get()).map_err(|_| StorageError::InvalidStoredValue { field: "saved-view revision" })?],
    ).map_err(|error| database_error(&error, "replace saved view"))
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
        <StorageWriter as SchedulePersistence>::delete_schedule(
            *self,
            schedule_id,
            expected_revision,
        )
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
                .execute(
                    "DELETE FROM one_time_schedule WHERE public_id = ?1",
                    [id.as_bytes().as_slice()],
                )
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
                .execute(
                    "DELETE FROM schedule_exception WHERE series_id = ?1",
                    [series_row_id],
                )
                .map_err(|error| database_error(&error, "delete schedule exceptions"))?;
            transaction
                .execute(
                    "DELETE FROM schedule_rule_segment WHERE series_id = ?1",
                    [series_row_id],
                )
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
        ScheduleId::OneTime(id) => insert_one_time_schedule(transaction, snapshot, &id.as_bytes()),
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
            let interval =
                HalfOpenInterval::try_new(UtcMicros::new(row.get(0)?), UtcMicros::new(row.get(1)?))
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
            if schedule_rule_conflicts_with_intervals(rule, &[candidate_interval]).map_err(
                |_| StorageError::InvalidStoredValue {
                    field: "schedule time-zone overlap validation",
                },
            )? {
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
                            value
                                .try_into()
                                .map_err(|_| StorageError::InvalidStoredValue {
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

fn setting_bool(value: i64, field: &'static str) -> Result<bool, StorageError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(StorageError::InvalidStoredValue { field }),
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
    let interval = rule
        .one_time_interval()
        .ok_or(StorageError::InvalidStoredValue {
            field: "one-time schedule rule",
        })?;
    let created_zone_id = rule
        .created_zone_id()
        .ok_or(StorageError::InvalidStoredValue {
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
    let resolved_zone_id =
        rule.time_zone_for_anchor_date(anchor_date)
            .ok_or(StorageError::InvalidStoredValue {
                field: "schedule exception rule segment",
            })?;
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

    /// Creates a verified full-fidelity SQLite online backup at a new user-selected path.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the destination already exists, the backup API fails, or
    /// SQLite quick/foreign-key verification rejects the completed image.
    pub fn create_backup(&mut self, destination: &Path) -> Result<(), StorageError> {
        create_user_backup(&*self.writer.writer.connection_mut(), destination)
    }

    /// Restores a previously verified full-fidelity SQLite backup into the current store.
    ///
    /// Callers must obtain explicit destructive confirmation and quiesce dependent work first.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when verification or the SQLite restore API fails.
    pub fn restore_backup(&mut self, source: &Path) -> Result<(), StorageError> {
        restore_user_backup(self.writer.writer.connection_mut(), source)
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

#[derive(Debug)]
struct CsvRecord {
    line: i64,
    fields: Vec<String>,
}

fn parse_csv_records(bytes: &[u8]) -> Result<Vec<CsvRecord>, StorageError> {
    let source = std::str::from_utf8(bytes).map_err(|_| StorageError::DataOperationFailed {
        operation: "decode CSV import",
    })?;
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut line = 1_i64;
    let mut record_line = 1_i64;
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                field.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(std::mem::take(&mut field));
            }
            '\n' if !quoted => {
                fields.push(std::mem::take(&mut field));
                records.push(CsvRecord {
                    line: record_line,
                    fields: std::mem::take(&mut fields),
                });
                line += 1;
                record_line = line;
            }
            '\n' => {
                field.push(character);
                line += 1;
            }
            '\r' if !quoted && characters.peek() == Some(&'\n') => {}
            other => field.push(other),
        }
    }
    if quoted {
        return Err(StorageError::DataOperationFailed {
            operation: "parse CSV import",
        });
    }
    if !field.is_empty() || !fields.is_empty() {
        fields.push(field);
        records.push(CsvRecord {
            line: record_line,
            fields,
        });
    }
    if records
        .first()
        .is_none_or(|record| record.fields.first().map(String::as_str) != Some("record_type"))
    {
        return Err(StorageError::DataOperationFailed {
            operation: "validate CSV import header",
        });
    }
    Ok(records)
}

fn validate_csv_record(record: &CsvRecord) -> Result<&[String], (Option<&str>, &'static str)> {
    let fields = record.fields.as_slice();
    if fields.len() != 12 {
        return Err((None, "invalid_column_count"));
    }
    if fields[1] != "1" {
        return Err((Some("format_version"), "unsupported_format_version"));
    }
    match fields[0].as_str() {
        "category" => {
            if hex_id_bytes(&fields[2]).is_some() && !fields[11].trim().is_empty() {
                Ok(fields)
            } else {
                Err((Some("stable_id"), "invalid_category"))
            }
        }
        "application" => {
            if hex_id_bytes(&fields[2]).is_none()
                || hex_id_bytes(&fields[9]).is_none()
                || (!fields[10].is_empty() && hex_id_bytes(&fields[10]).is_none())
                || fields[11].trim().is_empty()
            {
                Err((Some("stable_id"), "invalid_application"))
            } else {
                Ok(fields)
            }
        }
        "activity" => {
            let times_ok = fields[5].parse::<i64>().is_ok() && fields[6].parse::<i64>().is_ok();
            let codes_ok = fields[7]
                .parse::<i64>()
                .is_ok_and(|code| (0..=6).contains(&code))
                && fields[8]
                    .parse::<i64>()
                    .is_ok_and(|code| (0..=14).contains(&code));
            let application_ok = fields[9].is_empty() || hex_id_bytes(&fields[9]).is_some();
            if times_ok && codes_ok && application_ok && activity_tracker_id(&fields[2]).is_some() {
                Ok(fields)
            } else {
                Err((Some("activity"), "invalid_activity"))
            }
        }
        _ => Err((Some("record_type"), "unsupported_record_type")),
    }
}

fn insert_import_failure(
    transaction: &Transaction<'_>,
    batch_row_id: i64,
    failure: &ImportFailure,
) -> Result<(), StorageError> {
    let source_line =
        i64::try_from(failure.line()).map_err(|_| StorageError::InvalidStoredValue {
            field: "CSV import failure line",
        })?;
    transaction.execute(
        "INSERT INTO import_error(import_batch_id, source_line, field_name, error_code, summary) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![batch_row_id, source_line, failure.field(), failure.code(), "CSV row rejected"],
    ).map_err(|error| database_error(&error, "record CSV import error"))?;
    Ok(())
}

fn complete_cancelled_csv_import(
    transaction: &Transaction<'_>,
    request: &CsvImportRequest,
    batch_row_id: i64,
    parsed: u64,
    accepted: u64,
    completed_at_utc: UtcMicros,
    checkpoint: &str,
) -> Result<(), StorageError> {
    let rejected = parsed.saturating_sub(accepted);
    transaction
        .execute(
            "UPDATE import_batch SET state = 3, parsed_count = ?2, accepted_count = ?3,
             rejected_count = ?4, committed_count = 0, completed_utc_us = ?5 WHERE id = ?1",
            params![
                batch_row_id,
                as_i64(parsed, "cancelled CSV parsed count")?,
                as_i64(accepted, "cancelled CSV accepted count")?,
                as_i64(rejected, "cancelled CSV rejected count")?,
                completed_at_utc.get()
            ],
        )
        .map_err(|error| database_error(&error, "complete cancelled CSV import batch"))?;
    transaction
        .execute(
            "UPDATE job_record SET state = 3, progress_current = ?2, progress_total = ?2,
             safe_checkpoint = ?3, completed_utc_us = ?4 WHERE public_id = ?1",
            params![
                job_public_id(request.job_id().get()).as_slice(),
                as_i64(parsed, "cancelled CSV progress")?,
                checkpoint,
                completed_at_utc.get()
            ],
        )
        .map_err(|error| database_error(&error, "complete cancelled CSV import job"))?;
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the transactional category, application, then activity merge order is deliberately explicit"
)]
fn merge_csv_stage(
    transaction: &Transaction<'_>,
    revision: DataRevision,
    batch_row_id: i64,
    cancellation: &CancellationToken,
) -> Result<bool, StorageError> {
    let mut categories = transaction
        .prepare(
            "SELECT stable_id, display_name FROM import_stage_v1 WHERE record_type = 'category'",
        )
        .map_err(|error| database_error(&error, "read staged CSV categories"))?;
    let categories = categories
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| database_error(&error, "iterate staged CSV categories"))?;
    for category in categories {
        if cancellation.is_cancelled() {
            return Ok(true);
        }
        let (id, name) =
            category.map_err(|error| database_error(&error, "read staged CSV category"))?;
        let category_id = hex_id_bytes(&id).ok_or(StorageError::DataOperationFailed {
            operation: "decode staged CSV category",
        })?;
        transaction.execute("INSERT INTO category(public_id, display_name, archived, revision, updated_utc_us) VALUES (?1, ?2, 0, 0, 0) ON CONFLICT(public_id) DO UPDATE SET display_name = excluded.display_name", params![category_id.as_slice(), name])
            .map_err(|error| database_error(&error, "merge CSV category"))?;
    }
    let mut applications = transaction.prepare("SELECT stable_id, category_id, display_name FROM import_stage_v1 WHERE record_type = 'application'")
        .map_err(|error| database_error(&error, "read staged CSV applications"))?;
    let applications = applications
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| database_error(&error, "iterate staged CSV applications"))?;
    for application in applications {
        if cancellation.is_cancelled() {
            return Ok(true);
        }
        let (id, category, name) =
            application.map_err(|error| database_error(&error, "read staged CSV application"))?;
        let category_row: Option<i64> = if category.is_empty() {
            None
        } else {
            let category_id = hex_id_bytes(&category).ok_or(StorageError::DataOperationFailed {
                operation: "decode staged CSV application category",
            })?;
            transaction
                .query_row(
                    "SELECT id FROM category WHERE public_id = ?1",
                    [category_id.as_slice()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| database_error(&error, "find staged CSV category"))?
        };
        let application_id = hex_id_bytes(&id).ok_or(StorageError::DataOperationFailed {
            operation: "decode staged CSV application",
        })?;
        transaction.execute("INSERT INTO application(public_id, display_name, display_name_override, category_id, exclusion_policy, first_seen_utc_us, last_seen_utc_us, icon_digest) VALUES (?1, ?2, NULL, ?3, 0, 0, 0, NULL) ON CONFLICT(public_id) DO UPDATE SET display_name = excluded.display_name, category_id = excluded.category_id", params![application_id.as_slice(), name, category_row])
            .map_err(|error| database_error(&error, "merge CSV application"))?;
    }
    let mut statement = transaction.prepare(
        "SELECT source_line, stable_id, start_utc_us, end_utc_us, activity_state, activity_cause, application_id FROM import_stage_v1 WHERE record_type = 'activity' ORDER BY source_line",
    ).map_err(|error| database_error(&error, "read staged CSV activities"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| database_error(&error, "iterate staged CSV activities"))?;
    for row in rows {
        if cancellation.is_cancelled() {
            return Ok(true);
        }
        let (line, stable, start, end, state, cause, application) =
            row.map_err(|error| database_error(&error, "read staged CSV activity"))?;
        let tracker = activity_tracker_id(&stable).ok_or(StorageError::DataOperationFailed {
            operation: "decode staged CSV activity tracker",
        })?;
        let start = start
            .parse::<i64>()
            .map_err(|_| StorageError::DataOperationFailed {
                operation: "decode staged CSV activity start",
            })?;
        let end = end
            .parse::<i64>()
            .map_err(|_| StorageError::DataOperationFailed {
                operation: "decode staged CSV activity end",
            })?;
        let state = state
            .parse::<i64>()
            .map_err(|_| StorageError::DataOperationFailed {
                operation: "decode staged CSV activity state",
            })?;
        let cause = cause
            .parse::<i64>()
            .map_err(|_| StorageError::DataOperationFailed {
                operation: "decode staged CSV activity cause",
            })?;
        transaction.execute(
            "INSERT INTO tracker_run(public_id, started_utc_us, ended_utc_us, clean_end, platform_session_marker, adapter_version, end_evidence)
             VALUES (?1, ?2, NULL, 1, NULL, 'csv-import-v1', NULL) ON CONFLICT(public_id) DO NOTHING",
            params![tracker.as_slice(), start],
        ).map_err(|error| database_error(&error, "create CSV import tracker"))?;
        let tracker_row: i64 = transaction
            .query_row(
                "SELECT id FROM tracker_run WHERE public_id = ?1",
                [tracker.as_slice()],
                |row| row.get(0),
            )
            .map_err(|error| database_error(&error, "find CSV import tracker"))?;
        let application_row: Option<i64> = if application.is_empty() {
            None
        } else {
            let application_id =
                hex_id_bytes(&application).ok_or(StorageError::DataOperationFailed {
                    operation: "decode staged CSV activity application",
                })?;
            transaction
                .query_row(
                    "SELECT id FROM application WHERE public_id = ?1",
                    [application_id.as_slice()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| database_error(&error, "find CSV import application"))?
        };
        let exact_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM activity_interval WHERE tracker_run_id = ?1 AND start_utc_us = ?2 AND end_utc_us = ?3 AND state = ?4 AND cause = ?5 AND application_id IS ?6)",
            params![tracker_row, start, end, state, cause, application_row], |row| row.get::<_, i64>(0),
        ).map_err(|error| database_error(&error, "check CSV activity idempotency"))? != 0;
        if exact_exists {
            continue;
        }
        let overlap: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM activity_interval WHERE start_utc_us < ?1 AND end_utc_us > ?2)", params![end, start], |row| row.get::<_, i64>(0))
            .map_err(|error| database_error(&error, "check CSV activity overlap"))? != 0;
        if overlap {
            let failure = ImportFailure::try_new(
                line.cast_unsigned(),
                Some("activity".to_owned()),
                "overlapping_activity",
            )
            .map_err(|_| StorageError::DataOperationFailed {
                operation: "create CSV overlap failure",
            })?;
            insert_import_failure(transaction, batch_row_id, &failure)?;
            continue;
        }
        transaction.execute(
            "INSERT INTO activity_interval(tracker_run_id, start_utc_us, end_utc_us, state, cause, application_id, origin, uncertainty_us, source_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 0, ?7)",
            params![tracker_row, start, end, state, cause, application_row, nonnegative_i64(revision.get(), "CSV activity revision")?],
        ).map_err(|error| database_error(&error, "merge CSV activity"))?;
    }
    Ok(false)
}

fn hex_id_bytes(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut result = [0; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(result)
}

fn activity_tracker_id(stable_id: &str) -> Option<[u8; 16]> {
    stable_id.split(':').next().and_then(hex_id_bytes)
}

fn csv_fingerprint(bytes: &[u8]) -> [u8; 16] {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x9e37_79b9_7f4a_7c15_u64;
    for byte in bytes {
        first = (first ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        second = second.rotate_left(5) ^ u64::from(*byte);
    }
    let mut output = [0; 16];
    output[..8].copy_from_slice(&first.to_le_bytes());
    output[8..].copy_from_slice(&second.to_le_bytes());
    output
}

fn as_i64(value: u64, field: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidStoredValue { field })
}

fn job_public_id(value: u64) -> [u8; 16] {
    let mut id = [0; 16];
    id[..8].copy_from_slice(&value.to_le_bytes());
    id
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
        CancellationSource, CommandEnvelope, CommandId, CsvImportRequest, DataOperationDestination,
        EntityRevision, FocusCommand, FocusKind, FocusNotificationError, FocusNotificationPort,
        FocusPersistence, FocusService, FocusSnapshot, ImportBatchId, JobId, LayoutPersistence,
        LayoutSnapshot, OrderingKey, SavedViewId, SavedViewPersistence, SavedViewSnapshot,
        ScheduleId, SchedulePersistence, SchedulePersistenceError, ScheduleSnapshot,
        SchemaRevision, SettingsPersistence, SettingsPersistenceError, SettingsSnapshot,
        SettingsThemeMode, TitleObservationResult, TitleStabilizer, TrackingCheckpoint,
        TrackingPersistenceIntent, TrackingPersistencePort, TrackingPersistenceSubmit,
    };
    use openmanic_domain::{
        ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, Application,
        ApplicationId, ApplicationName, Category, CategoryId, CategoryName, FocusSessionId,
        FocusSessionState, HalfOpenInterval, LayoutDocument, OneTimeScheduleId,
        SavedViewDefinition, SavedViewFields, SavedViewRange, SavedViewRelativeRange, ScheduleRule,
        ScheduleSeriesId, TrackerRunId, UtcMicros,
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
    fn cancelled_csv_import_retains_terminal_metadata_without_merging_staged_entities() {
        let database = TemporaryDatabase::new("cancelled-csv-import");
        let source = database.path().with_extension("csv");
        fs::write(
            &source,
            "record_type,format_version,stable_id,related_id,aux_id,start_utc_us,end_utc_us,activity_state,activity_cause,application_id,category_id,display_name\ncategory,1,01010101010101010101010101010101,,,,,,,,,Imported\n",
        )
        .expect("write deterministic CSV fixture");
        let mut store = open_store(database.path(), 91);
        let request = CsvImportRequest::new(
            JobId::new(91),
            ImportBatchId::from_bytes([91; 16]),
            DataOperationDestination::new(source.clone()),
            openmanic_application::ImportDestinationScope::CurrentStore,
        );
        let (source_cancel, cancellation) = CancellationSource::new();
        let _ = source_cancel.cancel();

        assert_eq!(
            store
                .writer()
                .import_csv_cancellable(&request, UtcMicros::new(91), &cancellation),
            Ok(None)
        );

        let connection = Connection::open(database.path()).expect("open cancellation evidence");
        let (state, parsed, accepted, rejected, committed): (i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT state, parsed_count, accepted_count, rejected_count, committed_count FROM import_batch WHERE public_id = ?1",
                [request.batch_id().as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("read cancelled batch");
        assert_eq!(
            (state, parsed, accepted, rejected, committed),
            (3, 0, 0, 0, 0)
        );
        let category_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM category", [], |row| row.get(0))
            .expect("count categories after cancellation");
        assert_eq!(category_count, 0);
        drop(connection);
        let _ = fs::remove_file(source);
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "the one-time persistence contract is intentionally verified end-to-end in one fixture"
    )]
    fn schedule_persistence_writes_one_time_schedule_and_category_reference() {
        let database = TemporaryDatabase::new("schedule-one-time");
        let mut store = open_store(database.path(), 21);
        let category = Category::new(
            category_id(22),
            CategoryName::try_new("Planning").expect("valid category name"),
        );
        assert_eq!(
            store
                .writer()
                .create_category(&category, UtcMicros::new(10)),
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
            .query_row("SELECT exclusion_policy FROM application", [], |row| {
                row.get(0)
            })
            .expect("stored exclusion policy");
        assert_eq!(excluded, 1);
        assert_eq!(
            writer.set_applications_excluded(&[application_id(32)], false),
            Ok(revision(4))
        );
        let excluded: i64 = writer
            .writer
            .connection_mut()
            .query_row("SELECT exclusion_policy FROM application", [], |row| {
                row.get(0)
            })
            .expect("stored exclusion policy");
        assert_eq!(excluded, 0);
    }

    #[test]
    fn window_title_persistence_deduplicates_text_and_coalesces_adjacent_spans() {
        let database = TemporaryDatabase::new("window-title-spans");
        let mut store = open_store(database.path(), 33);
        let writer = store.writer();
        writer
            .register_tracker_run(&registration(33, 0))
            .expect("the tracker run should register");
        seed_active_application(writer, 34);
        let first = accepted_title(application_id(34), 0, 2_000_000, "Plan");
        let second = accepted_title(application_id(34), 2_000_000, 4_000_000, "Plan");
        let different = accepted_title(application_id(34), 4_000_000, 6_000_000, "Review");

        assert_eq!(
            writer.persist_window_title(run_id(33), &first),
            Ok(revision(4))
        );
        assert_eq!(
            writer.persist_window_title(run_id(33), &second),
            Ok(revision(5))
        );
        assert_eq!(
            writer.persist_window_title(run_id(33), &different),
            Ok(revision(6))
        );

        let spans: Vec<(String, i64, i64)> = writer
            .writer
            .connection_mut()
            .prepare(
                "SELECT title, start_utc_us, end_utc_us
                   FROM window_title_span
                   JOIN window_title_text ON window_title_text.id = window_title_span.title_text_id
                   ORDER BY start_utc_us",
            )
            .expect("title span query should prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("title span query should execute")
            .collect::<Result<_, _>>()
            .expect("title span rows should decode");
        assert_eq!(
            spans,
            vec![
                ("Plan".to_owned(), 0, 4_000_000),
                ("Review".to_owned(), 4_000_000, 6_000_000)
            ]
        );
        let distinct_title_texts: i64 = writer
            .writer
            .connection_mut()
            .query_row("SELECT COUNT(*) FROM window_title_text", [], |row| {
                row.get(0)
            })
            .expect("title text count should read");
        assert_eq!(distinct_title_texts, 2);
    }

    #[test]
    fn window_title_collection_requires_an_explicit_enabled_setting() {
        let database = TemporaryDatabase::new("window-title-setting");
        let mut store = open_store(database.path(), 35);
        let writer = store.writer();
        assert_eq!(writer.window_title_collection_enabled(), Ok(false));
        writer
            .writer
            .connection_mut()
            .execute(
                "INSERT INTO user_settings(
                    singleton_id, schema_version, first_launch_consent_revision,
                    start_tracking_automatically, start_at_login, close_to_tray,
                    idle_threshold_seconds, idle_policy, collect_window_titles,
                    time_zone_mode, manual_time_zone_id, theme_mode, density,
                    notifications_enabled, focus_sounds_enabled, tray_explanation_acknowledged,
                    revision, updated_utc_us
                 ) VALUES (1, 1, 0, 0, 0, 1, 60, 1, 1, 0, NULL, 0, 1, 1, 1, 0, 0, 0)",
                [],
            )
            .expect("enabled title collection setting should insert");
        assert_eq!(writer.window_title_collection_enabled(), Ok(true));
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
    fn saved_views_round_trip_full_definition_and_enforce_revisions() {
        let database = TemporaryDatabase::new("saved-views");
        let mut store = open_store(database.path(), 17);
        let id = SavedViewId::from_bytes([17; 16]);
        let document = openmanic_domain::SavedViewDocument::try_new(
            SavedViewDefinition {
                public_id: id.encoded(),
                name: "Weekly review".to_owned(),
                display_order: 0,
                range: SavedViewRange::Relative(SavedViewRelativeRange::Week),
                grouping: "category".to_owned(),
                filters: SavedViewFields {
                    schema_version: 1,
                    fields: Vec::new(),
                },
                sort: "duration-descending".to_owned(),
                widget_configuration: SavedViewFields {
                    schema_version: 1,
                    fields: Vec::new(),
                },
            },
            0,
        )
        .expect("fixture view should be valid");
        let snapshot = SavedViewSnapshot::try_new(id, document, EntityRevision::new(0), time(10))
            .expect("fixture identity should agree");
        assert_eq!(
            SavedViewPersistence::create_saved_view(store.writer(), &snapshot),
            Ok(revision(1))
        );
        let loaded =
            SavedViewPersistence::load_saved_views(store.writer()).expect("saved view should load");
        assert_eq!(loaded.invalid_count(), 0);
        assert_eq!(loaded.snapshots(), &[snapshot]);
        assert_eq!(
            SavedViewPersistence::delete_saved_view(store.writer(), id, EntityRevision::new(1)),
            Err(openmanic_application::SavedViewPersistenceError::RevisionConflict)
        );
        assert_eq!(
            SavedViewPersistence::delete_saved_view(store.writer(), id, EntityRevision::new(0)),
            Ok(revision(2))
        );
    }

    #[test]
    fn dashboard_layout_round_trips_atomically_and_rejects_stale_replacements() {
        let database = TemporaryDatabase::new("dashboard-layout");
        let mut store = open_store(database.path(), 18);
        let document = LayoutDocument::safe_default();
        let initial = LayoutSnapshot::new(document.clone(), EntityRevision::new(0), time(10));

        assert_eq!(LayoutPersistence::load_layout(store.writer()), Ok(None));
        assert_eq!(
            LayoutPersistence::replace_layout(store.writer(), &initial, None),
            Ok(revision(1))
        );
        let loaded = LayoutPersistence::load_layout(store.writer())
            .expect("stored layout should load")
            .expect("initial save should create the singleton layout");
        assert_eq!(loaded.document(), &document);
        assert_eq!(loaded.entity_revision(), EntityRevision::new(0));
        assert_eq!(loaded.updated_at_utc(), time(10));

        let replacement = LayoutSnapshot::new(document, EntityRevision::new(0), time(20));
        assert_eq!(
            LayoutPersistence::replace_layout(
                store.writer(),
                &replacement,
                Some(EntityRevision::new(0))
            ),
            Ok(revision(2))
        );
        assert_eq!(
            LayoutPersistence::replace_layout(store.writer(), &replacement, None),
            Err(openmanic_application::LayoutPersistenceError::RevisionConflict)
        );
        let updated = LayoutPersistence::load_layout(store.writer())
            .expect("stored replacement should load")
            .expect("layout remains present");
        assert_eq!(updated.entity_revision(), EntityRevision::new(1));
        assert_eq!(updated.updated_at_utc(), time(20));
        assert_eq!(updated.document().revision(), 1);
    }

    fn settings_snapshot(revision: EntityRevision) -> SettingsSnapshot {
        SettingsSnapshot::new(
            3,
            false,
            true,
            false,
            120,
            2,
            true,
            1,
            Some("Europe/Amsterdam".to_owned()),
            SettingsThemeMode::Light,
            3,
            false,
            false,
            true,
            revision,
        )
    }

    #[test]
    fn settings_replacement_is_complete_atomic_and_revision_guarded() {
        let database = TemporaryDatabase::new("settings-replacement");
        let mut store = open_store(database.path(), 36);
        let initial = settings_snapshot(EntityRevision::new(0));

        assert_eq!(SettingsPersistence::load_settings(store.writer()), Ok(None));
        assert_eq!(
            SettingsPersistence::replace_settings(store.writer(), &initial, None),
            Ok(revision(1))
        );
        assert_eq!(
            SettingsPersistence::load_settings(store.writer()),
            Ok(Some(initial.clone()))
        );

        let replacement = SettingsSnapshot::new(
            7,
            true,
            false,
            true,
            900,
            3,
            false,
            0,
            None,
            SettingsThemeMode::FollowSystem,
            4,
            true,
            true,
            false,
            EntityRevision::new(0),
        );
        assert_eq!(
            SettingsPersistence::replace_settings(
                store.writer(),
                &replacement,
                Some(EntityRevision::new(0)),
            ),
            Ok(revision(2))
        );
        let expected_replacement = SettingsSnapshot::new(
            replacement.consent_revision(),
            replacement.start_tracking_automatically(),
            replacement.start_at_login(),
            replacement.close_to_tray(),
            replacement.idle_threshold_seconds(),
            replacement.idle_policy_code(),
            replacement.collect_window_titles(),
            replacement.time_zone_mode(),
            replacement.manual_time_zone_id().map(str::to_owned),
            replacement.theme_mode(),
            replacement.density_code(),
            replacement.notifications_enabled(),
            replacement.focus_sounds_enabled(),
            replacement.tray_explanation_acknowledged(),
            EntityRevision::new(1),
        );
        assert_eq!(
            SettingsPersistence::load_settings(store.writer()),
            Ok(Some(expected_replacement))
        );
        assert_eq!(
            SettingsPersistence::replace_settings(store.writer(), &replacement, None),
            Err(SettingsPersistenceError::RevisionConflict)
        );
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

    fn accepted_title(
        application_id: ApplicationId,
        stable_since_utc: i64,
        accepted_at_utc: i64,
        text: &str,
    ) -> openmanic_application::AcceptedWindowTitle {
        let mut stabilizer = TitleStabilizer::default();
        assert!(matches!(
            stabilizer.observe(application_id, time(stable_since_utc), text, true, false),
            TitleObservationResult::Ignored
        ));
        match stabilizer.observe(application_id, time(accepted_at_utc), text, true, false) {
            TitleObservationResult::Accepted(title) => title,
            TitleObservationResult::Ignored => unreachable!("fixture title should be stable"),
        }
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
