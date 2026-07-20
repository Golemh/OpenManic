//! Read repositories that map strict SQLite rows into storage-owned snapshots.

use std::path::Path;

use openmanic_application::{DataRevision, EntityRevision, ScheduleId, ScheduleSnapshot};
use openmanic_domain::{
    ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, Application, ApplicationId,
    ApplicationName, Category, CategoryId, CategoryName, HalfOpenInterval, PowerTransitionEvidence,
    OneTimeScheduleId, ScheduleRule, ScheduleSeriesId, TrackerRunId, UtcMicros,
};
use rusqlite::Transaction;

use crate::{SqliteReader, StorageError};
use crate::writer::load_schedule_series_rule;

/// One immutable activity interval returned by a storage snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityRecord {
    raw_id: u64,
    interval: ActivityInterval,
    recovered: bool,
    uncertainty_us: u64,
    source_revision: DataRevision,
}

impl ActivityRecord {
    /// Returns the stable SQLite row identity used by timeline hit testing and detail recovery.
    #[must_use]
    pub const fn raw_id(&self) -> u64 {
        self.raw_id
    }

    /// Returns the canonical domain interval reconstructed from the strict row.
    #[must_use]
    pub const fn interval(&self) -> ActivityInterval {
        self.interval
    }

    /// Returns whether recovery, rather than live tracking, created this interval.
    #[must_use]
    pub const fn recovered(&self) -> bool {
        self.recovered
    }

    /// Returns the explicit nonnegative uncertainty duration in microseconds.
    #[must_use]
    pub const fn uncertainty_us(&self) -> u64 {
        self.uncertainty_us
    }

    /// Returns the atomic store revision that committed this interval.
    #[must_use]
    pub const fn source_revision(&self) -> DataRevision {
        self.source_revision
    }
}

/// One immutable application row returned by a storage snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationRecord {
    application: Application,
    excluded: bool,
}

impl ApplicationRecord {
    /// Returns the current domain application value.
    #[must_use]
    pub fn application(&self) -> &Application {
        &self.application
    }

    /// Returns whether future foreground evidence for this application is excluded.
    #[must_use]
    pub const fn excluded(&self) -> bool {
        self.excluded
    }
}

/// One immutable category row returned by a storage snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryRecord {
    category: Category,
}

/// One immutable schedule fact returned by a storage snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleRecord {
    snapshot: ScheduleSnapshot,
}

impl ScheduleRecord {
    /// Returns the immutable schedule, including its stable ID and optimistic revision.
    #[must_use]
    pub const fn snapshot(&self) -> &ScheduleSnapshot {
        &self.snapshot
    }
}

impl CategoryRecord {
    /// Returns the current domain category value.
    #[must_use]
    pub fn category(&self) -> &Category {
        &self.category
    }
}

/// Correlated immutable facts read at one committed store revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadSnapshot {
    revision: DataRevision,
    activities: Vec<ActivityRecord>,
    applications: Vec<ApplicationRecord>,
    categories: Vec<CategoryRecord>,
    schedules: Vec<ScheduleRecord>,
}

impl ReadSnapshot {
    /// Returns the revision read from the same transaction as every returned fact.
    #[must_use]
    pub const fn revision(&self) -> DataRevision {
        self.revision
    }

    /// Returns canonical activity intervals ordered by start and row identity.
    #[must_use]
    pub fn activities(&self) -> &[ActivityRecord] {
        &self.activities
    }

    /// Returns applications ordered by their current display name and stable ID.
    #[must_use]
    pub fn applications(&self) -> &[ApplicationRecord] {
        &self.applications
    }

    /// Returns categories ordered by their current display name and stable ID.
    #[must_use]
    pub fn categories(&self) -> &[CategoryRecord] {
        &self.categories
    }

    /// Returns all personal schedules ordered by stable identity within their schedule form.
    #[must_use]
    pub fn schedules(&self) -> &[ScheduleRecord] {
        &self.schedules
    }
}

/// A short, query-only SQLite session that returns correlated immutable snapshots.
pub struct SqliteReadSession {
    reader: SqliteReader,
}

impl SqliteReadSession {
    pub(crate) fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            reader: SqliteReader::open(path)?,
        })
    }

    /// Reads the store revision and its activity, application, and category facts atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if SQLite cannot create a short read transaction or a strict row
    /// cannot be mapped into the storage snapshot contract.
    pub fn snapshot(&mut self) -> Result<ReadSnapshot, StorageError> {
        let transaction = self
            .reader
            .connection()
            .unchecked_transaction()
            .map_err(|error| database_error(&error, "begin read snapshot"))?;
        let snapshot = read_snapshot(&transaction)?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit read snapshot"))?;
        Ok(snapshot)
    }
}

pub(crate) struct ActivityRepository;

impl ActivityRepository {
    fn read(transaction: &Transaction<'_>) -> Result<Vec<ActivityRecord>, StorageError> {
        let mut statement = transaction
            .prepare(
                "SELECT activity_interval.id, tracker_run.public_id, activity_interval.start_utc_us,
                        activity_interval.end_utc_us, activity_interval.state,
                        activity_interval.cause, application.public_id,
                        activity_interval.origin, activity_interval.uncertainty_us,
                        activity_interval.source_revision
                   FROM activity_interval
                   JOIN tracker_run ON tracker_run.id = activity_interval.tracker_run_id
              LEFT JOIN application ON application.id = activity_interval.application_id
               ORDER BY activity_interval.start_utc_us, activity_interval.id",
            )
            .map_err(|error| database_error(&error, "prepare activity snapshot"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })
            .map_err(|error| database_error(&error, "read activity snapshot"))?;
        rows.map(|row| {
            let (
                raw_id,
                tracker_run_id,
                start_utc_us,
                end_utc_us,
                state,
                cause,
                application_id,
                origin,
                uncertainty_us,
                source_revision,
            ) = row.map_err(|error| database_error(&error, "read activity snapshot"))?;
            Ok(ActivityRecord {
                raw_id: u64::try_from(raw_id).map_err(|_| StorageError::InvalidStoredValue {
                    field: "activity row identity",
                })?,
                interval: activity_interval(
                    tracker_run_id,
                    start_utc_us,
                    end_utc_us,
                    state,
                    cause,
                    application_id,
                )?,
                recovered: origin == 2,
                uncertainty_us: u64::try_from(uncertainty_us).map_err(|_| {
                    StorageError::InvalidStoredValue {
                        field: "activity uncertainty",
                    }
                })?,
                source_revision: data_revision(source_revision)?,
            })
        })
        .collect()
    }
}

pub(crate) struct ApplicationRepository;

impl ApplicationRepository {
    fn read(transaction: &Transaction<'_>) -> Result<Vec<ApplicationRecord>, StorageError> {
        let mut statement = transaction
            .prepare(
                "SELECT application.public_id, application.display_name, category.public_id,
                        application.first_seen_utc_us, application.last_seen_utc_us,
                        application.exclusion_policy
                   FROM application
              LEFT JOIN category ON category.id = application.category_id
               ORDER BY application.display_name, application.public_id",
            )
            .map_err(|error| database_error(&error, "prepare application snapshot"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| database_error(&error, "read application snapshot"))?;
        rows.map(|row| {
            let (
                id,
                display_name,
                category_id,
                first_seen_utc_us,
                last_seen_utc_us,
                exclusion_policy,
            ) = row.map_err(|error| database_error(&error, "read application snapshot"))?;
            Ok(ApplicationRecord {
                application: Application::try_new(
                    ApplicationId::from_bytes(fixed_id(id, "application ID")?),
                    ApplicationName::try_new(display_name).map_err(|_| {
                        StorageError::InvalidStoredValue {
                            field: "application display name",
                        }
                    })?,
                    category_id
                        .map(|value| fixed_id(value, "category ID"))
                        .transpose()?
                        .map(CategoryId::from_bytes),
                    UtcMicros::new(first_seen_utc_us),
                    UtcMicros::new(last_seen_utc_us),
                )
                .map_err(|_| StorageError::InvalidStoredValue {
                    field: "application observation range",
                })?,
                excluded: match exclusion_policy {
                    0 => false,
                    1 => true,
                    _ => {
                        return Err(StorageError::InvalidStoredValue {
                            field: "application exclusion policy",
                        });
                    }
                },
            })
        })
        .collect()
    }
}

pub(crate) struct CategoryRepository;

impl CategoryRepository {
    fn read(transaction: &Transaction<'_>) -> Result<Vec<CategoryRecord>, StorageError> {
        let mut statement = transaction
            .prepare(
                "SELECT public_id, display_name FROM category ORDER BY display_name, public_id",
            )
            .map_err(|error| database_error(&error, "prepare category snapshot"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| database_error(&error, "read category snapshot"))?;
        rows.map(|row| {
            let (id, display_name) =
                row.map_err(|error| database_error(&error, "read category snapshot"))?;
            Ok(CategoryRecord {
                category: Category::new(
                    CategoryId::from_bytes(fixed_id(id, "category ID")?),
                    CategoryName::try_new(display_name).map_err(|_| {
                        StorageError::InvalidStoredValue {
                            field: "category display name",
                        }
                    })?,
                ),
            })
        })
        .collect()
    }
}

pub(crate) struct ScheduleRepository;

impl ScheduleRepository {
    fn read(transaction: &Transaction<'_>) -> Result<Vec<ScheduleRecord>, StorageError> {
        let mut schedules = Self::read_one_time(transaction)?;
        schedules.extend(Self::read_series(transaction)?);
        Ok(schedules)
    }

    fn read_one_time(transaction: &Transaction<'_>) -> Result<Vec<ScheduleRecord>, StorageError> {
        let mut statement = transaction
            .prepare(
                "SELECT one_time_schedule.public_id, one_time_schedule.label, category.public_id,
                        one_time_schedule.start_utc_us, one_time_schedule.end_utc_us,
                        one_time_schedule.created_zone_id, one_time_schedule.created_utc_us,
                        one_time_schedule.revision
                   FROM one_time_schedule
              LEFT JOIN category ON category.id = one_time_schedule.category_id
               ORDER BY one_time_schedule.public_id",
            )
            .map_err(|error| database_error(&error, "prepare one-time schedule snapshot"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?, row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?, row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?, row.get::<_, i64>(7)?,
                ))
            })
            .map_err(|error| database_error(&error, "read one-time schedule snapshot"))?;
        rows.map(|row| {
            let (id, label, category_id, start, end, zone, created, revision) =
                row.map_err(|error| database_error(&error, "read one-time schedule snapshot"))?;
            let interval = HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
                .map_err(|_| StorageError::InvalidStoredValue { field: "one-time schedule interval" })?;
            let rule = ScheduleRule::one_time(
                label,
                category_id.map(|value| fixed_id(value, "schedule category ID")).transpose()?.map(CategoryId::from_bytes),
                interval,
                zone,
            ).map_err(|_| StorageError::InvalidStoredValue { field: "one-time schedule rule" })?;
            Ok(ScheduleRecord { snapshot: ScheduleSnapshot::try_new(
                ScheduleId::OneTime(OneTimeScheduleId::from_bytes(fixed_id(id, "one-time schedule ID")?)),
                rule,
                EntityRevision::new(u64::try_from(revision).map_err(|_| StorageError::InvalidStoredValue { field: "one-time schedule revision" })?),
                UtcMicros::new(created),
            ).map_err(|_| StorageError::InvalidStoredValue { field: "one-time schedule snapshot" })? })
        }).collect()
    }

    fn read_series(transaction: &Transaction<'_>) -> Result<Vec<ScheduleRecord>, StorageError> {
        let mut statement = transaction
            .prepare("SELECT id, public_id, created_utc_us, revision FROM schedule_series ORDER BY public_id")
            .map_err(|error| database_error(&error, "prepare schedule-series snapshot"))?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)))
            .map_err(|error| database_error(&error, "read schedule-series snapshot"))?;
        rows.map(|row| {
            let (row_id, id, created, revision) = row.map_err(|error| database_error(&error, "read schedule-series snapshot"))?;
            Ok(ScheduleRecord { snapshot: ScheduleSnapshot::try_new(
                ScheduleId::Series(ScheduleSeriesId::from_bytes(fixed_id(id, "schedule series ID")?)),
                load_schedule_series_rule(transaction, row_id)?,
                EntityRevision::new(u64::try_from(revision).map_err(|_| StorageError::InvalidStoredValue { field: "schedule series revision" })?),
                UtcMicros::new(created),
            ).map_err(|_| StorageError::InvalidStoredValue { field: "schedule series snapshot" })? })
        }).collect()
    }
}

/// Reads one complete authoritative schedule snapshot inside an existing transaction.
pub(crate) fn read_schedule_snapshot(
    transaction: &Transaction<'_>,
    schedule_id: ScheduleId,
) -> Result<Option<ScheduleSnapshot>, StorageError> {
    Ok(ScheduleRepository::read(transaction)?
        .into_iter()
        .map(|record| record.snapshot)
        .find(|snapshot| snapshot.id() == schedule_id))
}

pub(crate) fn read_snapshot(transaction: &Transaction<'_>) -> Result<ReadSnapshot, StorageError> {
    let revision = transaction
        .query_row(
            "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| database_error(&error, "read store revision"))?;
    Ok(ReadSnapshot {
        revision: data_revision(revision)?,
        activities: ActivityRepository::read(transaction)?,
        applications: ApplicationRepository::read(transaction)?,
        categories: CategoryRepository::read(transaction)?,
        schedules: ScheduleRepository::read(transaction)?,
    })
}

fn fixed_id(value: Vec<u8>, field: &'static str) -> Result<[u8; 16], StorageError> {
    value
        .try_into()
        .map_err(|_| StorageError::InvalidStoredValue { field })
}

fn data_revision(value: i64) -> Result<DataRevision, StorageError> {
    u64::try_from(value)
        .map(DataRevision::new)
        .map_err(|_| StorageError::InvalidStoredValue {
            field: "data revision",
        })
}

fn activity_interval(
    tracker_run_id: Vec<u8>,
    start_utc_us: i64,
    end_utc_us: i64,
    state: i64,
    cause: i64,
    application_id: Option<Vec<u8>>,
) -> Result<ActivityInterval, StorageError> {
    let range = HalfOpenInterval::try_new(UtcMicros::new(start_utc_us), UtcMicros::new(end_utc_us))
        .map_err(|_| StorageError::InvalidStoredValue {
            field: "activity interval range",
        })?;
    let state = activity_state(state)?;
    let cause = activity_cause(cause)?;
    let evidence = if state == ActivityState::PoweredOff {
        let boundaries =
            PowerTransitionEvidence::try_new(range.start(), range.end()).map_err(|_| {
                StorageError::InvalidStoredValue {
                    field: "powered-off evidence",
                }
            })?;
        if cause != ActivityCause::ConfirmedShutdown {
            return Err(StorageError::InvalidStoredValue {
                field: "powered-off cause",
            });
        }
        ActivityEvidence::confirmed_shutdown(boundaries)
    } else {
        ActivityEvidence::try_from_cause(cause).map_err(|_| StorageError::InvalidStoredValue {
            field: "activity evidence",
        })?
    };
    ActivityInterval::try_new(
        TrackerRunId::from_bytes(fixed_id(tracker_run_id, "tracker run ID")?),
        range,
        state,
        evidence,
        application_id
            .map(|value| fixed_id(value, "application ID"))
            .transpose()?
            .map(ApplicationId::from_bytes),
    )
    .map_err(|_| StorageError::InvalidStoredValue {
        field: "activity interval invariant",
    })
}

fn activity_state(value: i64) -> Result<ActivityState, StorageError> {
    match value {
        0 => Ok(ActivityState::Active),
        1 => Ok(ActivityState::Idle),
        2 => Ok(ActivityState::PausedByUser),
        3 => Ok(ActivityState::Excluded),
        4 => Ok(ActivityState::Unavailable),
        5 => Ok(ActivityState::PoweredOff),
        6 => Ok(ActivityState::UnknownMissing),
        _ => Err(StorageError::InvalidStoredValue {
            field: "activity state",
        }),
    }
}

fn activity_cause(value: i64) -> Result<ActivityCause, StorageError> {
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
            field: "activity cause",
        }),
    }
}

pub(crate) fn database_error(error: &rusqlite::Error, operation: &'static str) -> StorageError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            StorageError::Busy { operation }
        }
        _ => StorageError::DatabaseOperation { operation },
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use openmanic_domain::{
        Application, ApplicationId, ApplicationName, Category, CategoryId, CategoryName, UtcMicros,
    };

    use super::read_snapshot;
    use crate::{SqliteStore, StoreOpenOptions};

    static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDatabase {
        path: PathBuf,
    }

    impl TemporaryDatabase {
        fn new() -> Self {
            let sequence = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            let filename = format!(
                "openmanic-om220-snapshot-{}-{sequence}.sqlite3",
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
    fn wal_reader_snapshot_stays_correlated_while_writer_commits_new_revision() {
        let database = TemporaryDatabase::new();
        let mut store =
            SqliteStore::open(database.path(), &StoreOpenOptions::new([31; 16], 0, "test"))
                .expect("the isolated store should open");
        store
            .writer()
            .upsert_category(&category(1, "First"), UtcMicros::new(0))
            .expect("the first category should commit");

        let mut reader = store
            .open_read_session()
            .expect("the query-only session should open");
        let transaction = reader
            .reader
            .connection()
            .unchecked_transaction()
            .expect("the reader should begin a short snapshot transaction");
        let before = read_snapshot(&transaction).expect("the snapshot should read");

        store
            .writer()
            .upsert_category(&category(2, "Second"), UtcMicros::new(1))
            .expect("the WAL writer should commit while the reader is open");
        let during = read_snapshot(&transaction).expect("the open snapshot should remain readable");
        assert_eq!(during.revision(), before.revision());
        assert_eq!(during.categories(), before.categories());
        transaction
            .commit()
            .expect("the reader snapshot should finish promptly");

        let after = reader
            .snapshot()
            .expect("a new short read transaction should observe the writer commit");
        assert!(after.revision() > before.revision());
        assert_eq!(after.categories().len(), 2);
    }

    #[test]
    fn snapshot_exposes_the_committed_application_exclusion_policy() {
        let database = TemporaryDatabase::new();
        let mut store =
            SqliteStore::open(database.path(), &StoreOpenOptions::new([32; 16], 0, "test"))
                .expect("the isolated store should open");
        let application_id = ApplicationId::from_bytes([4; 16]);
        let application = Application::try_new(
            application_id,
            ApplicationName::try_new("Browser").expect("fixture application name should be valid"),
            None,
            UtcMicros::new(0),
            UtcMicros::new(0),
        )
        .expect("fixture application should be valid");
        let writer = store.writer();
        writer
            .upsert_application(&application)
            .expect("the fixture application should commit");
        writer
            .set_applications_excluded(&[application_id], true)
            .expect("the exclusion policy should commit");
        let snapshot = store
            .open_read_session()
            .expect("the reader should open")
            .snapshot()
            .expect("the reader should observe the committed policy");
        assert!(snapshot.applications()[0].excluded());
    }

    fn category(byte: u8, name: &str) -> Category {
        Category::new(
            CategoryId::from_bytes([byte; 16]),
            CategoryName::try_new(name).expect("fixture category name should be valid"),
        )
    }

    fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
        let mut sidecar = OsString::from(path.as_os_str());
        sidecar.push(suffix);
        PathBuf::from(sidecar)
    }
}
