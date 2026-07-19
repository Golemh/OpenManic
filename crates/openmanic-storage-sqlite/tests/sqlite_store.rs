//! End-to-end SQLite storage initialization and schema-invariant checks.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use openmanic_storage_sqlite::{
    JournalMode, LATEST_SCHEMA_VERSION, SqliteReader, SqliteWriter, StorageError, StoreOpenOptions,
    SynchronousMode,
};
use rusqlite::{Connection, ErrorCode, params};

static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

struct TemporaryDatabase {
    path: PathBuf,
}

impl TemporaryDatabase {
    fn new(case_name: &str) -> Self {
        let identifier = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
        let filename = format!(
            "openmanic-om150-{case_name}-{}-{identifier}.sqlite3",
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

fn sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = OsString::from(database_path.as_os_str());
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

fn open_options(store_byte: u8) -> StoreOpenOptions {
    StoreOpenOptions::new([store_byte; 16], 1_725_000_000_000_000, "0.1.0-test")
}

#[test]
fn writer_and_reader_verify_required_connection_modes() -> Result<(), Box<dyn Error>> {
    let database = TemporaryDatabase::new("connection-modes");
    let options = open_options(1);

    {
        let writer = SqliteWriter::open(database.path(), &options)?;
        let configuration = writer.configuration();
        assert_eq!(configuration.journal_mode(), Some(JournalMode::Wal));
        assert_eq!(configuration.synchronous(), Some(SynchronousMode::Full));
        assert!(configuration.foreign_keys());
        assert!(!configuration.trusted_schema());
        assert!(!configuration.query_only());
        assert_eq!(writer.schema_version()?, LATEST_SCHEMA_VERSION);
    }

    let reader = SqliteReader::open(database.path())?;
    let configuration = reader.configuration();
    assert_eq!(configuration.journal_mode(), None);
    assert_eq!(configuration.synchronous(), None);
    assert!(configuration.foreign_keys());
    assert!(!configuration.trusted_schema());
    assert!(configuration.query_only());
    assert_eq!(reader.schema_version()?, LATEST_SCHEMA_VERSION);

    Ok(())
}

#[test]
fn migration_ledger_rejects_a_checksum_mismatch() -> Result<(), Box<dyn Error>> {
    let database = TemporaryDatabase::new("checksum-mismatch");
    let options = open_options(2);

    SqliteWriter::open(database.path(), &options)?;
    let connection = Connection::open(database.path())?;
    connection.execute(
        "UPDATE schema_migration SET checksum = ?1 WHERE version = 1",
        [vec![0_u8; 8]],
    )?;
    drop(connection);

    let Err(error) = SqliteWriter::open(database.path(), &options) else {
        return Err("a mismatched migration checksum was accepted".into());
    };
    assert_eq!(
        error,
        StorageError::MigrationChecksumMismatch { version: 1 }
    );

    Ok(())
}

#[test]
fn migration_refuses_existing_tables_without_a_ledger() -> Result<(), Box<dyn Error>> {
    let database = TemporaryDatabase::new("missing-ledger");
    let connection = Connection::open(database.path())?;
    connection.execute_batch("CREATE TABLE unrelated_prior_schema (id INTEGER PRIMARY KEY)")?;
    drop(connection);

    let Err(error) = SqliteWriter::open(database.path(), &open_options(9)) else {
        return Err("an existing schema without a migration ledger was accepted".into());
    };
    assert_eq!(error, StorageError::MigrationLedgerMissing);

    Ok(())
}

#[test]
fn migration_ledger_rejects_a_database_newer_than_the_binary() -> Result<(), Box<dyn Error>> {
    let database = TemporaryDatabase::new("newer-version");
    let options = open_options(3);

    SqliteWriter::open(database.path(), &options)?;
    let connection = Connection::open(database.path())?;
    connection.execute(
        "UPDATE store_metadata SET schema_version = ?1 WHERE singleton_id = 1",
        [i64::from(LATEST_SCHEMA_VERSION + 1)],
    )?;
    connection.pragma_update(None, "user_version", LATEST_SCHEMA_VERSION + 1)?;
    drop(connection);

    let Err(error) = SqliteWriter::open(database.path(), &options) else {
        return Err("a newer database schema was accepted".into());
    };
    assert_eq!(
        error,
        StorageError::DatabaseNewerThanBinary {
            database_version: LATEST_SCHEMA_VERSION + 1,
            supported_version: LATEST_SCHEMA_VERSION,
        }
    );

    Ok(())
}

#[test]
fn initial_schema_is_strict_and_enforces_key_constraints_and_indexes() -> Result<(), Box<dyn Error>>
{
    let database = TemporaryDatabase::new("schema-invariants");
    SqliteWriter::open(database.path(), &open_options(4))?;
    let connection = Connection::open(database.path())?;

    assert_strict_tables(&connection)?;
    assert_key_constraints(&connection)?;
    assert_required_indexes(&connection)?;

    Ok(())
}

fn assert_strict_tables(connection: &Connection) -> Result<(), Box<dyn Error>> {
    let strict_tables = [
        "schema_migration",
        "store_metadata",
        "category",
        "application",
        "application_identity",
        "window_title_text",
        "tracker_run",
        "activity_interval",
        "open_activity_checkpoint",
        "window_title_span",
        "focus_session",
        "one_time_schedule",
        "schedule_series",
        "schedule_rule_segment",
        "schedule_exception",
        "dashboard_layout",
        "saved_overview_view",
        "user_settings",
        "job_record",
        "import_batch",
        "import_error",
    ];
    for table_name in strict_tables {
        let strict: i64 = connection.query_row(
            "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND name = ?1",
            [table_name],
            |row| row.get(0),
        )?;
        assert_eq!(strict, 1, "{table_name} must be a STRICT table");
    }

    Ok(())
}

fn assert_key_constraints(connection: &Connection) -> Result<(), Box<dyn Error>> {
    let invalid_category = connection.execute(
        "INSERT INTO category(
             public_id, display_name, productivity_class, created_utc_us, updated_utc_us
         ) VALUES (?1, ?2, 0, 0, 0)",
        params![vec![5_u8; 16], vec![0_u8]],
    );
    assert!(invalid_category.is_err());

    connection.execute(
        "INSERT INTO focus_session(
             public_id, kind, state, intended_duration_us, actual_start_utc_us,
             deadline_utc_us, revision
         ) VALUES (?1, 0, 1, 1, 10, 11, 0)",
        [vec![6_u8; 16]],
    )?;
    let second_active_session = connection.execute(
        "INSERT INTO focus_session(
             public_id, kind, state, intended_duration_us, actual_start_utc_us,
             paused_remaining_us, revision
         ) VALUES (?1, 0, 2, 1, 12, 1, 0)",
        [vec![7_u8; 16]],
    );
    let Err(error) = second_active_session else {
        return Err("the second active or paused focus session was accepted".into());
    };
    assert_eq!(
        error.sqlite_error_code(),
        Some(ErrorCode::ConstraintViolation)
    );

    connection.execute(
        "INSERT INTO tracker_run(
             public_id, started_utc_us, ended_utc_us, clean_end, adapter_version
         ) VALUES (?1, 0, NULL, 0, 'test-adapter')",
        [vec![8_u8; 16]],
    )?;
    let active_interval_without_application = connection.execute(
        "INSERT INTO activity_interval(
             tracker_run_id, start_utc_us, end_utc_us, state, cause, application_id,
             origin, uncertainty_us, source_revision
         ) VALUES (1, 0, 1, 0, 0, NULL, 0, 0, 0)",
        [],
    );
    assert!(active_interval_without_application.is_err());

    Ok(())
}

fn assert_required_indexes(connection: &Connection) -> Result<(), Box<dyn Error>> {
    let mut statement =
        connection.prepare("SELECT name FROM sqlite_master WHERE type = 'index'")?;
    let index_names = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for index_name in [
        "idx_activity_interval_start",
        "idx_activity_interval_application_start",
        "idx_window_title_span_application_start",
        "idx_application_category_display_name",
        "idx_schedule_rule_segment_series_effective_dates",
        "idx_focus_session_actual_start",
        "idx_saved_overview_view_display_order",
    ] {
        assert!(
            index_names
                .iter()
                .any(|found_name| found_name == index_name),
            "missing required index: {index_name}"
        );
    }

    Ok(())
}
