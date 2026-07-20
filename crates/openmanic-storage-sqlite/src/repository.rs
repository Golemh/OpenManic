//! Read repositories that map strict SQLite rows into storage-owned snapshots.

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use openmanic_application::{
    CsvExportRequest, DataOperationOutcome, DataRevision, EntityRevision, FocusKind, FocusSnapshot,
    SavedViewId, SavedViewLoad, SavedViewSnapshot, ScheduleId, ScheduleSnapshot,
};
use openmanic_domain::{
    ActivityCause, ActivityEvidence, ActivityInterval, ActivityState, Application, ApplicationId,
    ApplicationName, Category, CategoryId, CategoryName, FocusSessionId, FocusSessionState,
    HalfOpenInterval, LayoutDefinition, LayoutDocument, LayoutField, LayoutFields, LayoutHeight,
    LayoutScalar, LayoutWidgetDefinition, OneTimeScheduleId, PowerTransitionEvidence,
    SavedViewDefinition, SavedViewField, SavedViewFields, SavedViewRange, SavedViewRelativeRange,
    SavedViewScalar, SavedViewTimeZoneBehavior, ScheduleRule, ScheduleSeriesId, TrackerRunId,
    UtcMicros,
};
use rusqlite::Transaction;

use crate::writer::load_schedule_series_rule;
use crate::{SqliteReader, StorageError};

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

/// One durable focus-session fact returned by a correlated storage snapshot.
///
/// `interval` is present only when persisted lifecycle columns establish both canonical bounds.
/// In particular, a paused session has no fabricated end based on its remaining duration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusRecord {
    snapshot: FocusSnapshot,
    interval: Option<HalfOpenInterval>,
}

impl FocusRecord {
    /// Returns the complete restored focus-session state.
    #[must_use]
    pub const fn snapshot(&self) -> &FocusSnapshot {
        &self.snapshot
    }

    /// Returns the authoritative interval when durable state supplies both endpoints.
    #[must_use]
    pub const fn interval(&self) -> Option<HalfOpenInterval> {
        self.interval
    }
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
    focus_sessions: Vec<FocusRecord>,
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

    /// Returns durable focus sessions in stable row order from the same read transaction.
    #[must_use]
    pub fn focus_sessions(&self) -> &[FocusRecord] {
        &self.focus_sessions
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

    /// Streams a versioned CSV interchange file from one correlated read transaction.
    ///
    /// The method holds only one SQLite row at a time while writing; it never materializes a
    /// full dashboard snapshot or exposes a SQLite row identity as an interchange identifier.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the destination, SQLite reader, timestamp conversion, or
    /// stream write fails.
    pub fn export_csv(
        &mut self,
        request: &CsvExportRequest,
        completed_at_utc: UtcMicros,
    ) -> Result<DataOperationOutcome, StorageError> {
        let file = File::create(request.destination().path()).map_err(|_| {
            StorageError::DataOperationFailed {
                operation: "create CSV export",
            }
        })?;
        let mut output = BufWriter::new(file);
        let transaction = self
            .reader
            .connection()
            .unchecked_transaction()
            .map_err(|error| database_error(&error, "begin CSV export"))?;
        let revision = transaction
            .query_row(
                "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| database_error(&error, "read CSV export revision"))?;
        let revision = data_revision(revision)?;
        write_csv_record(
            &mut output,
            &[
                "record_type",
                "format_version",
                "stable_id",
                "start_utc",
                "end_utc",
                "start_utc_us",
                "end_utc_us",
                "activity_state",
                "activity_cause",
                "application_id",
                "category_id",
                "display_name",
            ],
        )?;
        let mut count = 0_u64;
        count = count.saturating_add(export_categories(&transaction, &mut output)?);
        count = count.saturating_add(export_applications(&transaction, &mut output)?);
        count = count.saturating_add(export_activities(&transaction, request, &mut output)?);
        output
            .flush()
            .map_err(|_| StorageError::DataOperationFailed {
                operation: "flush CSV export",
            })?;
        transaction
            .commit()
            .map_err(|error| database_error(&error, "commit CSV export read"))?;
        Ok(DataOperationOutcome::new(count, revision, completed_at_utc))
    }
}

fn export_categories(
    transaction: &Transaction<'_>,
    output: &mut impl Write,
) -> Result<u64, StorageError> {
    let mut statement = transaction
        .prepare("SELECT public_id, display_name FROM category ORDER BY public_id")
        .map_err(|error| database_error(&error, "prepare CSV categories"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| database_error(&error, "query CSV categories"))?;
    let mut count = 0_u64;
    while let Some(row) = rows
        .next()
        .map_err(|error| database_error(&error, "read CSV category"))?
    {
        let id = fixed_id(
            row.get::<_, Vec<u8>>(0)
                .map_err(|error| database_error(&error, "read CSV category ID"))?,
            "CSV category ID",
        )?;
        let name: String = row
            .get(1)
            .map_err(|error| database_error(&error, "read CSV category name"))?;
        write_csv_record(
            output,
            &[
                "category",
                "1",
                &hex_id(id),
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                &name,
            ],
        )?;
        count = count.saturating_add(1);
    }
    Ok(count)
}

fn export_applications(
    transaction: &Transaction<'_>,
    output: &mut impl Write,
) -> Result<u64, StorageError> {
    let mut statement = transaction
        .prepare(
            "SELECT application.public_id, application.display_name, category.public_id
               FROM application
          LEFT JOIN category ON category.id = application.category_id
           ORDER BY application.public_id",
        )
        .map_err(|error| database_error(&error, "prepare CSV applications"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| database_error(&error, "query CSV applications"))?;
    let mut count = 0_u64;
    while let Some(row) = rows
        .next()
        .map_err(|error| database_error(&error, "read CSV application"))?
    {
        let id = fixed_id(
            row.get::<_, Vec<u8>>(0)
                .map_err(|error| database_error(&error, "read CSV application ID"))?,
            "CSV application ID",
        )?;
        let name: String = row
            .get(1)
            .map_err(|error| database_error(&error, "read CSV application name"))?;
        let category_id = row
            .get::<_, Option<Vec<u8>>>(2)
            .map_err(|error| database_error(&error, "read CSV application category"))?
            .map(|value| fixed_id(value, "CSV application category ID").map(hex_id))
            .transpose()?;
        write_csv_record(
            output,
            &[
                "application",
                "1",
                &hex_id(id),
                "",
                "",
                "",
                "",
                "",
                "",
                &hex_id(id),
                category_id.as_deref().unwrap_or(""),
                &name,
            ],
        )?;
        count = count.saturating_add(1);
    }
    Ok(count)
}

fn export_activities(
    transaction: &Transaction<'_>,
    request: &CsvExportRequest,
    output: &mut impl Write,
) -> Result<u64, StorageError> {
    let mut statement = transaction
        .prepare(
            "SELECT tracker_run.public_id, activity_interval.start_utc_us, activity_interval.end_utc_us,
                    activity_interval.state, activity_interval.cause, application.public_id
               FROM activity_interval
               JOIN tracker_run ON tracker_run.id = activity_interval.tracker_run_id
          LEFT JOIN application ON application.id = activity_interval.application_id
              WHERE activity_interval.end_utc_us > ?1 AND activity_interval.start_utc_us < ?2
           ORDER BY activity_interval.start_utc_us, activity_interval.id",
        )
        .map_err(|error| database_error(&error, "prepare CSV activities"))?;
    let mut rows = statement
        .query(rusqlite::params![
            request.range().start().get(),
            request.range().end().get()
        ])
        .map_err(|error| database_error(&error, "query CSV activities"))?;
    let mut count = 0_u64;
    while let Some(row) = rows
        .next()
        .map_err(|error| database_error(&error, "read CSV activity"))?
    {
        let tracker = fixed_id(
            row.get::<_, Vec<u8>>(0)
                .map_err(|error| database_error(&error, "read CSV activity tracker"))?,
            "CSV activity tracker ID",
        )?;
        let start: i64 = row
            .get(1)
            .map_err(|error| database_error(&error, "read CSV activity start"))?;
        let end: i64 = row
            .get(2)
            .map_err(|error| database_error(&error, "read CSV activity end"))?;
        let state: i64 = row
            .get(3)
            .map_err(|error| database_error(&error, "read CSV activity state"))?;
        let cause: i64 = row
            .get(4)
            .map_err(|error| database_error(&error, "read CSV activity cause"))?;
        let application_id = row
            .get::<_, Option<Vec<u8>>>(5)
            .map_err(|error| database_error(&error, "read CSV activity application"))?
            .map(|value| fixed_id(value, "CSV activity application ID").map(hex_id))
            .transpose()?;
        let start_utc = rfc3339(start)?;
        let end_utc = rfc3339(end)?;
        let stable_id = format!("{}:{start}:{end}:{state}:{cause}", hex_id(tracker));
        write_csv_record(
            output,
            &[
                "activity",
                "1",
                &stable_id,
                &start_utc,
                &end_utc,
                &start.to_string(),
                &end.to_string(),
                &state.to_string(),
                &cause.to_string(),
                application_id.as_deref().unwrap_or(""),
                "",
                "",
            ],
        )?;
        count = count.saturating_add(1);
    }
    Ok(count)
}

fn rfc3339(micros: i64) -> Result<String, StorageError> {
    jiff::Timestamp::from_microsecond(micros)
        .map(|timestamp| timestamp.to_string())
        .map_err(|_| StorageError::InvalidStoredValue {
            field: "CSV UTC instant",
        })
}

fn hex_id(id: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(32);
    for byte in id {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

fn write_csv_record(output: &mut impl Write, fields: &[&str]) -> Result<(), StorageError> {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            output
                .write_all(b",")
                .map_err(|_| StorageError::DataOperationFailed {
                    operation: "write CSV export",
                })?;
        }
        let needs_quotes = field.contains([',', '"', '\n', '\r']);
        if needs_quotes {
            output
                .write_all(b"\"")
                .map_err(|_| StorageError::DataOperationFailed {
                    operation: "write CSV export",
                })?;
        }
        for character in field.chars() {
            if character == '"' {
                output
                    .write_all(b"\"\"")
                    .map_err(|_| StorageError::DataOperationFailed {
                        operation: "write CSV export",
                    })?;
            } else {
                let mut encoded = [0_u8; 4];
                output
                    .write_all(character.encode_utf8(&mut encoded).as_bytes())
                    .map_err(|_| StorageError::DataOperationFailed {
                        operation: "write CSV export",
                    })?;
            }
        }
        if needs_quotes {
            output
                .write_all(b"\"")
                .map_err(|_| StorageError::DataOperationFailed {
                    operation: "write CSV export",
                })?;
        }
    }
    output
        .write_all(b"\n")
        .map_err(|_| StorageError::DataOperationFailed {
            operation: "write CSV export",
        })
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

pub(crate) struct FocusRepository;

impl FocusRepository {
    #[expect(
        clippy::too_many_lines,
        reason = "one transaction-scoped reader keeps the focus SQL row layout and validation adjacent"
    )]
    fn read(transaction: &Transaction<'_>) -> Result<Vec<FocusRecord>, StorageError> {
        let mut statement = transaction
            .prepare(
                "SELECT focus_session.public_id, focus_session.kind, focus_session.label,
                        category.public_id, focus_session.intended_duration_us, focus_session.state,
                        focus_session.planned_start_utc_us, focus_session.planned_end_utc_us,
                        focus_session.actual_start_utc_us, focus_session.deadline_utc_us,
                        focus_session.paused_remaining_us, focus_session.completed_utc_us,
                        focus_session.cancelled_utc_us, focus_session.revision
                   FROM focus_session LEFT JOIN category ON category.id = focus_session.category_id
               ORDER BY focus_session.id",
            )
            .map_err(|error| database_error(&error, "prepare focus snapshot"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, i64>(13)?,
                ))
            })
            .map_err(|error| database_error(&error, "read focus snapshot"))?;
        rows.map(|row| {
            let (
                id,
                kind,
                label,
                category_id,
                duration,
                state,
                planned_start,
                planned_end,
                actual_start,
                deadline,
                paused_remaining,
                completed,
                cancelled,
                revision,
            ) = row.map_err(|error| database_error(&error, "read focus snapshot"))?;
            let state = focus_state_from_columns((
                state,
                planned_start,
                planned_end,
                actual_start,
                deadline,
                paused_remaining,
                completed,
                cancelled,
            ))?;
            let snapshot = FocusSnapshot::try_restore(
                FocusSessionId::from_bytes(fixed_id(id, "focus stable ID")?),
                match kind {
                    0 => FocusKind::Focus,
                    1 => FocusKind::ShortBreak,
                    _ => {
                        return Err(StorageError::InvalidStoredValue {
                            field: "focus kind",
                        });
                    }
                },
                label,
                duration,
                category_id
                    .map(|id| fixed_id(id, "focus category ID"))
                    .transpose()?
                    .map(CategoryId::from_bytes),
                state,
                EntityRevision::new(u64::try_from(revision).map_err(|_| {
                    StorageError::InvalidStoredValue {
                        field: "focus revision",
                    }
                })?),
            )
            .map_err(|_| StorageError::InvalidStoredValue {
                field: "focus session",
            })?;
            Ok(FocusRecord {
                interval: focus_interval(snapshot.session().state())?,
                snapshot,
            })
        })
        .collect()
    }
}

fn focus_interval(state: FocusSessionState) -> Result<Option<HalfOpenInterval>, StorageError> {
    let bounds = match state {
        FocusSessionState::Planned {
            planned_start,
            planned_end,
        } => Some((planned_start, planned_end)),
        FocusSessionState::Running {
            started_at,
            deadline,
        } => Some((started_at, deadline)),
        FocusSessionState::Completed {
            started_at,
            completed_at,
        } => Some((started_at, completed_at)),
        FocusSessionState::Cancelled {
            started_at,
            cancelled_at,
        } => Some((started_at, cancelled_at)),
        FocusSessionState::Ready | FocusSessionState::Paused { .. } => None,
    };
    bounds
        .map(|(start, end)| {
            HalfOpenInterval::try_new(start, end).map_err(|_| StorageError::InvalidStoredValue {
                field: "focus interval",
            })
        })
        .transpose()
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
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .map_err(|error| database_error(&error, "read one-time schedule snapshot"))?;
        rows.map(|row| {
            let (id, label, category_id, start, end, zone, created, revision) =
                row.map_err(|error| database_error(&error, "read one-time schedule snapshot"))?;
            let interval = HalfOpenInterval::try_new(UtcMicros::new(start), UtcMicros::new(end))
                .map_err(|_| StorageError::InvalidStoredValue {
                    field: "one-time schedule interval",
                })?;
            let rule = ScheduleRule::one_time(
                label,
                category_id
                    .map(|value| fixed_id(value, "schedule category ID"))
                    .transpose()?
                    .map(CategoryId::from_bytes),
                interval,
                zone,
            )
            .map_err(|_| StorageError::InvalidStoredValue {
                field: "one-time schedule rule",
            })?;
            Ok(ScheduleRecord {
                snapshot: ScheduleSnapshot::try_new(
                    ScheduleId::OneTime(OneTimeScheduleId::from_bytes(fixed_id(
                        id,
                        "one-time schedule ID",
                    )?)),
                    rule,
                    EntityRevision::new(u64::try_from(revision).map_err(|_| {
                        StorageError::InvalidStoredValue {
                            field: "one-time schedule revision",
                        }
                    })?),
                    UtcMicros::new(created),
                )
                .map_err(|_| StorageError::InvalidStoredValue {
                    field: "one-time schedule snapshot",
                })?,
            })
        })
        .collect()
    }

    fn read_series(transaction: &Transaction<'_>) -> Result<Vec<ScheduleRecord>, StorageError> {
        let mut statement = transaction
            .prepare("SELECT id, public_id, created_utc_us, revision FROM schedule_series ORDER BY public_id")
            .map_err(|error| database_error(&error, "prepare schedule-series snapshot"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| database_error(&error, "read schedule-series snapshot"))?;
        rows.map(|row| {
            let (row_id, id, created, revision) =
                row.map_err(|error| database_error(&error, "read schedule-series snapshot"))?;
            Ok(ScheduleRecord {
                snapshot: ScheduleSnapshot::try_new(
                    ScheduleId::Series(ScheduleSeriesId::from_bytes(fixed_id(
                        id,
                        "schedule series ID",
                    )?)),
                    load_schedule_series_rule(transaction, row_id)?,
                    EntityRevision::new(u64::try_from(revision).map_err(|_| {
                        StorageError::InvalidStoredValue {
                            field: "schedule series revision",
                        }
                    })?),
                    UtcMicros::new(created),
                )
                .map_err(|_| StorageError::InvalidStoredValue {
                    field: "schedule series snapshot",
                })?,
            })
        })
        .collect()
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

/// Restores every valid saved view while retaining a count of corrupt rows for diagnostics.
pub(crate) fn load_saved_views(
    connection: &mut rusqlite::Connection,
) -> Result<SavedViewLoad, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT public_id, name, display_order, schema_version, revision, definition_json, created_utc_us
               FROM saved_overview_view
              ORDER BY display_order, public_id",
        )
        .map_err(|error| database_error(&error, "prepare saved-view load"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|error| database_error(&error, "read saved views"))?;
    let mut snapshots = Vec::new();
    let mut invalid_count = 0;
    for row in rows {
        let Ok((id, name, display_order, schema_version, revision, definition, created)) = row
        else {
            invalid_count += 1;
            continue;
        };
        let snapshot = (|| {
            let id = SavedViewId::from_bytes(fixed_id(id, "saved-view stable ID")?);
            let display_order =
                u32::try_from(display_order).map_err(|_| StorageError::InvalidStoredValue {
                    field: "saved-view display order",
                })?;
            let schema_version =
                u16::try_from(schema_version).map_err(|_| StorageError::InvalidStoredValue {
                    field: "saved-view schema version",
                })?;
            let revision =
                u64::try_from(revision).map_err(|_| StorageError::InvalidStoredValue {
                    field: "saved-view revision",
                })?;
            let document = decode_saved_view_definition(&definition, revision)?;
            if document.schema_version() != schema_version
                || document.name() != name
                || document.display_order() != display_order
            {
                return Err(StorageError::InvalidStoredValue {
                    field: "saved-view normalized columns",
                });
            }
            SavedViewSnapshot::try_new(
                id,
                document,
                EntityRevision::new(revision),
                UtcMicros::new(created),
            )
            .map_err(|_| StorageError::InvalidStoredValue {
                field: "saved-view identity",
            })
        })();
        match snapshot {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(_) => invalid_count += 1,
        }
    }
    Ok(SavedViewLoad::new(snapshots, invalid_count))
}

pub(crate) fn encode_saved_view_definition(definition: &SavedViewDefinition) -> String {
    let mut output = String::from("OMSV1");
    push_saved_view_part(&mut output, &definition.public_id);
    push_saved_view_part(&mut output, &definition.name);
    push_saved_view_part(&mut output, &definition.display_order.to_string());
    match &definition.range {
        SavedViewRange::Relative(range) => {
            push_saved_view_part(&mut output, "relative");
            push_saved_view_part(
                &mut output,
                match range {
                    SavedViewRelativeRange::Day => "day",
                    SavedViewRelativeRange::Week => "week",
                    SavedViewRelativeRange::Month => "month",
                    SavedViewRelativeRange::Year => "year",
                    SavedViewRelativeRange::Custom => "custom",
                },
            );
        }
        SavedViewRange::Fixed {
            start_local_date,
            end_local_date,
            time_zone_behavior,
        } => {
            push_saved_view_part(&mut output, "fixed");
            push_saved_view_part(&mut output, start_local_date);
            push_saved_view_part(&mut output, end_local_date);
            match time_zone_behavior {
                SavedViewTimeZoneBehavior::Automatic => {
                    push_saved_view_part(&mut output, "automatic");
                }
                SavedViewTimeZoneBehavior::Manual(zone) => {
                    push_saved_view_part(&mut output, "manual");
                    push_saved_view_part(&mut output, zone);
                }
            }
        }
    }
    push_saved_view_part(&mut output, &definition.grouping);
    encode_saved_view_fields(&mut output, &definition.filters);
    push_saved_view_part(&mut output, &definition.sort);
    encode_saved_view_fields(&mut output, &definition.widget_configuration);
    output
}

fn push_saved_view_part(output: &mut String, part: &str) {
    output.push('|');
    output.push_str(&part.len().to_string());
    output.push(':');
    output.push_str(part);
}

fn encode_saved_view_fields(output: &mut String, fields: &SavedViewFields) {
    push_saved_view_part(output, &fields.schema_version.to_string());
    push_saved_view_part(output, &fields.fields.len().to_string());
    for field in &fields.fields {
        push_saved_view_part(output, &field.name);
        match &field.value {
            SavedViewScalar::Boolean(value) => {
                push_saved_view_part(output, "boolean");
                push_saved_view_part(output, if *value { "true" } else { "false" });
            }
            SavedViewScalar::Integer(value) => {
                push_saved_view_part(output, "integer");
                push_saved_view_part(output, &value.to_string());
            }
            SavedViewScalar::Text(value) => {
                push_saved_view_part(output, "text");
                push_saved_view_part(output, value);
            }
        }
    }
}

fn decode_saved_view_definition(
    source: &str,
    revision: u64,
) -> Result<openmanic_domain::SavedViewDocument, StorageError> {
    let mut parts = SavedViewParts::new(source)?;
    let public_id = parts.next()?;
    let name = parts.next()?;
    let display_order = parse_saved_view_number(&parts.next()?, "saved-view display order")?;
    let range = match parts.next()?.as_str() {
        "relative" => SavedViewRange::Relative(match parts.next()?.as_str() {
            "day" => SavedViewRelativeRange::Day,
            "week" => SavedViewRelativeRange::Week,
            "month" => SavedViewRelativeRange::Month,
            "year" => SavedViewRelativeRange::Year,
            "custom" => SavedViewRelativeRange::Custom,
            _ => return invalid_saved_view(),
        }),
        "fixed" => {
            let start_local_date = parts.next()?;
            let end_local_date = parts.next()?;
            let time_zone_behavior = match parts.next()?.as_str() {
                "automatic" => SavedViewTimeZoneBehavior::Automatic,
                "manual" => SavedViewTimeZoneBehavior::Manual(parts.next()?),
                _ => return invalid_saved_view(),
            };
            SavedViewRange::Fixed {
                start_local_date,
                end_local_date,
                time_zone_behavior,
            }
        }
        _ => return invalid_saved_view(),
    };
    let grouping = parts.next()?;
    let filters = decode_saved_view_fields(&mut parts)?;
    let sort = parts.next()?;
    let widget_configuration = decode_saved_view_fields(&mut parts)?;
    if !parts.finished() {
        return invalid_saved_view();
    }
    openmanic_domain::SavedViewDocument::try_new(
        SavedViewDefinition {
            public_id,
            name,
            display_order,
            range,
            grouping,
            filters,
            sort,
            widget_configuration,
        },
        revision,
    )
    .map_err(|_| StorageError::InvalidStoredValue {
        field: "saved-view document",
    })
}

fn decode_saved_view_fields(
    parts: &mut SavedViewParts<'_>,
) -> Result<SavedViewFields, StorageError> {
    let schema_version = parse_saved_view_number(&parts.next()?, "saved-view field schema")?;
    let count: usize = parse_saved_view_number(&parts.next()?, "saved-view field count")?;
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        let name = parts.next()?;
        let value = match parts.next()?.as_str() {
            "boolean" => match parts.next()?.as_str() {
                "true" => SavedViewScalar::Boolean(true),
                "false" => SavedViewScalar::Boolean(false),
                _ => return invalid_saved_view(),
            },
            "integer" => SavedViewScalar::Integer(parse_saved_view_number(
                &parts.next()?,
                "saved-view integer",
            )?),
            "text" => SavedViewScalar::Text(parts.next()?),
            _ => return invalid_saved_view(),
        };
        fields.push(SavedViewField { name, value });
    }
    Ok(SavedViewFields {
        schema_version,
        fields,
    })
}

fn parse_saved_view_number<T: std::str::FromStr>(
    value: &str,
    field: &'static str,
) -> Result<T, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidStoredValue { field })
}

fn invalid_saved_view<T>() -> Result<T, StorageError> {
    Err(StorageError::InvalidStoredValue {
        field: "saved-view document",
    })
}

struct SavedViewParts<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> SavedViewParts<'a> {
    fn new(source: &'a str) -> Result<Self, StorageError> {
        if !source.starts_with("OMSV1") {
            return invalid_saved_view();
        }
        Ok(Self { source, offset: 5 })
    }
    fn next(&mut self) -> Result<String, StorageError> {
        let remainder = self
            .source
            .get(self.offset..)
            .ok_or(StorageError::InvalidStoredValue {
                field: "saved-view document",
            })?;
        let remainder = remainder
            .strip_prefix('|')
            .ok_or(StorageError::InvalidStoredValue {
                field: "saved-view document",
            })?;
        let (length, value) =
            remainder
                .split_once(':')
                .ok_or(StorageError::InvalidStoredValue {
                    field: "saved-view document",
                })?;
        let length: usize = parse_saved_view_number(length, "saved-view part length")?;
        if value.len() < length || !value.is_char_boundary(length) {
            return invalid_saved_view();
        }
        self.offset = self.source.len() - value.len() + length;
        Ok(value[..length].to_owned())
    }
    fn finished(&self) -> bool {
        self.offset == self.source.len()
    }
}

pub(crate) fn encode_layout_definition(definition: &LayoutDefinition) -> String {
    let mut output = String::from("OMLY1");
    push_layout_part(&mut output, &definition.widgets.len().to_string());
    for widget in &definition.widgets {
        push_layout_part(&mut output, &widget.instance_id);
        push_layout_part(&mut output, &widget.kind_id);
        push_layout_part(&mut output, &widget.kind_schema_version.to_string());
        push_layout_part(&mut output, &widget.order.to_string());
        push_layout_part(&mut output, &widget.width_span.to_string());
        push_layout_part(
            &mut output,
            match widget.height {
                LayoutHeight::Compact => "compact",
                LayoutHeight::Standard => "standard",
                LayoutHeight::Tall => "tall",
            },
        );
        encode_layout_fields(&mut output, &widget.configuration);
        match &widget.appearance_overrides {
            Some(overrides) => {
                push_layout_part(&mut output, "overrides");
                encode_layout_fields(&mut output, overrides);
            }
            None => push_layout_part(&mut output, "no-overrides"),
        }
    }
    output
}

pub(crate) fn decode_layout_document(
    source: &str,
    revision: u64,
) -> Result<LayoutDocument, StorageError> {
    let mut parts = LayoutParts::new(source)?;
    let count: usize = parse_layout_number(&parts.next()?, "layout widget count")?;
    let mut widgets = Vec::with_capacity(count);
    for _ in 0..count {
        let instance_id = parts.next()?;
        let kind_id = parts.next()?;
        let kind_schema_version = parse_layout_number(&parts.next()?, "layout kind schema")?;
        let order = parse_layout_number(&parts.next()?, "layout widget order")?;
        let width_span = parse_layout_number(&parts.next()?, "layout width span")?;
        let height = match parts.next()?.as_str() {
            "compact" => LayoutHeight::Compact,
            "standard" => LayoutHeight::Standard,
            "tall" => LayoutHeight::Tall,
            _ => return invalid_layout(),
        };
        let configuration = decode_layout_fields(&mut parts)?;
        let appearance_overrides = match parts.next()?.as_str() {
            "overrides" => Some(decode_layout_fields(&mut parts)?),
            "no-overrides" => None,
            _ => return invalid_layout(),
        };
        widgets.push(LayoutWidgetDefinition {
            instance_id,
            kind_id,
            kind_schema_version,
            order,
            width_span,
            height,
            configuration,
            appearance_overrides,
        });
    }
    if !parts.finished() {
        return invalid_layout();
    }
    LayoutDocument::try_new(LayoutDefinition { widgets }, revision).map_err(|_| {
        StorageError::InvalidStoredValue {
            field: "layout document",
        }
    })
}

fn push_layout_part(output: &mut String, part: &str) {
    output.push('|');
    output.push_str(&part.len().to_string());
    output.push(':');
    output.push_str(part);
}

fn encode_layout_fields(output: &mut String, fields: &LayoutFields) {
    push_layout_part(output, &fields.schema_version.to_string());
    push_layout_part(output, &fields.fields.len().to_string());
    for field in &fields.fields {
        push_layout_part(output, &field.name);
        match &field.value {
            LayoutScalar::Boolean(value) => {
                push_layout_part(output, "boolean");
                push_layout_part(output, if *value { "true" } else { "false" });
            }
            LayoutScalar::Integer(value) => {
                push_layout_part(output, "integer");
                push_layout_part(output, &value.to_string());
            }
            LayoutScalar::Text(value) => {
                push_layout_part(output, "text");
                push_layout_part(output, value);
            }
        }
    }
}

fn decode_layout_fields(parts: &mut LayoutParts<'_>) -> Result<LayoutFields, StorageError> {
    let schema_version = parse_layout_number(&parts.next()?, "layout field schema")?;
    let count: usize = parse_layout_number(&parts.next()?, "layout field count")?;
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        let name = parts.next()?;
        let value = match parts.next()?.as_str() {
            "boolean" => match parts.next()?.as_str() {
                "true" => LayoutScalar::Boolean(true),
                "false" => LayoutScalar::Boolean(false),
                _ => return invalid_layout(),
            },
            "integer" => {
                LayoutScalar::Integer(parse_layout_number(&parts.next()?, "layout integer")?)
            }
            "text" => LayoutScalar::Text(parts.next()?),
            _ => return invalid_layout(),
        };
        fields.push(LayoutField { name, value });
    }
    Ok(LayoutFields {
        schema_version,
        fields,
    })
}

fn parse_layout_number<T: std::str::FromStr>(
    value: &str,
    field: &'static str,
) -> Result<T, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidStoredValue { field })
}

fn invalid_layout<T>() -> Result<T, StorageError> {
    Err(StorageError::InvalidStoredValue {
        field: "layout document",
    })
}

struct LayoutParts<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> LayoutParts<'a> {
    fn new(source: &'a str) -> Result<Self, StorageError> {
        if !source.starts_with("OMLY1") {
            return invalid_layout();
        }
        Ok(Self { source, offset: 5 })
    }

    fn next(&mut self) -> Result<String, StorageError> {
        let remainder = self
            .source
            .get(self.offset..)
            .ok_or(StorageError::InvalidStoredValue {
                field: "layout document",
            })?;
        let remainder = remainder
            .strip_prefix('|')
            .ok_or(StorageError::InvalidStoredValue {
                field: "layout document",
            })?;
        let (length, value) =
            remainder
                .split_once(':')
                .ok_or(StorageError::InvalidStoredValue {
                    field: "layout document",
                })?;
        let length: usize = parse_layout_number(length, "layout part length")?;
        if value.len() < length || !value.is_char_boundary(length) {
            return invalid_layout();
        }
        self.offset = self.source.len() - value.len() + length;
        Ok(value[..length].to_owned())
    }

    fn finished(&self) -> bool {
        self.offset == self.source.len()
    }
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
        focus_sessions: FocusRepository::read(transaction)?,
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

    use openmanic_application::{
        CsvExportRequest, DataOperationDestination, JobId, TitleDisclosure,
    };
    use openmanic_domain::{
        Application, ApplicationId, ApplicationName, Category, CategoryId, CategoryName, UtcMicros,
    };

    use super::{read_snapshot, write_csv_record};
    use crate::{SqliteStore, StoreOpenOptions};

    static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn csv_writer_quotes_delimiters_newlines_and_quotes_deterministically() {
        let mut output = Vec::new();
        write_csv_record(
            &mut output,
            &["plain", "comma,value", "quote\"value", "line\nbreak"],
        )
        .expect("in-memory CSV output must be writable");
        assert_eq!(
            String::from_utf8(output).expect("CSV fixture is UTF-8"),
            "plain,\"comma,value\",\"quote\"\"value\",\"line\nbreak\"\n"
        );
    }

    #[test]
    fn csv_export_streams_deterministic_category_and_application_rows() {
        let database = TemporaryDatabase::new();
        let destination = database.path().with_extension("csv");
        let mut store =
            SqliteStore::open(database.path(), &StoreOpenOptions::new([34; 16], 0, "test"))
                .expect("the isolated store should open");
        let category = category(8, "Personal, projects");
        store
            .writer()
            .upsert_category(&category, UtcMicros::new(0))
            .expect("the fixture category should commit");
        let application = Application::try_new(
            ApplicationId::from_bytes([9; 16]),
            ApplicationName::try_new("Browser").expect("fixture application name should validate"),
            Some(category.id()),
            UtcMicros::new(0),
            UtcMicros::new(0),
        )
        .expect("fixture application should validate");
        store
            .writer()
            .upsert_application(&application)
            .expect("the fixture application should commit");
        let request = CsvExportRequest::new(
            JobId::new(1),
            openmanic_domain::HalfOpenInterval::try_new(UtcMicros::new(0), UtcMicros::new(1))
                .expect("positive fixture range"),
            DataOperationDestination::new(destination.clone()),
            TitleDisclosure::Exclude,
        );

        let outcome = store
            .open_read_session()
            .expect("the reader should open")
            .export_csv(&request, UtcMicros::new(2))
            .expect("the CSV export should complete");
        let contents = fs::read_to_string(&destination).expect("the CSV should be readable");
        assert_eq!(outcome.row_count(), 2);
        assert!(contents.starts_with("record_type,format_version,stable_id,start_utc"));
        assert!(contents.contains("category,1,08080808080808080808080808080808"));
        assert!(contents.contains("application,1,09090909090909090909090909090909"));
        assert!(contents.contains("\"Personal, projects\""));
        let _ = fs::remove_file(destination);
    }

    #[test]
    fn user_backup_and_restore_use_verified_online_sqlite_images() {
        let database = TemporaryDatabase::new();
        let backup = database.path().with_extension("backup.sqlite3");
        let mut store =
            SqliteStore::open(database.path(), &StoreOpenOptions::new([35; 16], 0, "test"))
                .expect("the isolated store should open");
        store
            .writer()
            .upsert_category(&category(10, "Before backup"), UtcMicros::new(0))
            .expect("the backup fixture category should commit");
        store
            .create_backup(&backup)
            .expect("the online backup should verify");
        store
            .writer()
            .upsert_category(&category(11, "After backup"), UtcMicros::new(1))
            .expect("the post-backup category should commit");
        store
            .restore_backup(&backup)
            .expect("the verified backup should restore");

        let snapshot = store
            .open_read_session()
            .expect("the restored reader should open")
            .snapshot()
            .expect("the restored store should read");
        assert_eq!(snapshot.categories().len(), 1);
        assert_eq!(
            snapshot.categories()[0].category().name().as_str(),
            "Before backup"
        );
        let _ = fs::remove_file(backup);
    }

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
