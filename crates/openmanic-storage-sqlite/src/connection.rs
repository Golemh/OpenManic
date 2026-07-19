//! Private SQLite connection owners with verified writer and reader settings.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::errors::ConnectionSetting;
use crate::migration;
use crate::{StorageError, StoreOpenOptions};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const BUSY_TIMEOUT_MILLISECONDS: i64 = 5_000;
const SQLITE_SYNCHRONOUS_FULL: i64 = 2;

/// The verified mode for a SQLite connection owned by this crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionConfiguration {
    journal_mode: Option<JournalMode>,
    synchronous: Option<SynchronousMode>,
    foreign_keys: bool,
    trusted_schema: bool,
    query_only: bool,
    busy_timeout: Duration,
}

impl ConnectionConfiguration {
    /// Returns the verified journal mode when this connection owns writer configuration.
    #[must_use]
    pub const fn journal_mode(self) -> Option<JournalMode> {
        self.journal_mode
    }

    /// Returns the verified synchronous mode when this connection owns writer configuration.
    #[must_use]
    pub const fn synchronous(self) -> Option<SynchronousMode> {
        self.synchronous
    }

    /// Returns whether foreign-key enforcement is verified on this connection.
    #[must_use]
    pub const fn foreign_keys(self) -> bool {
        self.foreign_keys
    }

    /// Returns whether trusted-schema mode is enabled on this connection.
    #[must_use]
    pub const fn trusted_schema(self) -> bool {
        self.trusted_schema
    }

    /// Returns whether the connection is restricted to query-only access.
    #[must_use]
    pub const fn query_only(self) -> bool {
        self.query_only
    }

    /// Returns the bounded busy timeout verified for this connection.
    #[must_use]
    pub const fn busy_timeout(self) -> Duration {
        self.busy_timeout
    }
}

/// The SQLite journal mode required for OpenManic writer connections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalMode {
    /// Write-ahead logging, verified after configuration.
    Wal,
}

/// The SQLite durability mode required for OpenManic writer connections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynchronousMode {
    /// Full durability for committed user and tracking state.
    Full,
}

/// The crate-owned serialized SQLite writer connection.
///
/// Its raw SQLite connection remains private so callers cannot bypass the
/// migration, revision, and transaction policies owned by this crate.
pub struct SqliteWriter {
    connection: Connection,
    configuration: ConnectionConfiguration,
}

impl SqliteWriter {
    /// Opens, configures, verifies, and migrates a local SQLite writer connection.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the database cannot be opened safely, a
    /// required connection setting cannot be verified, the migration ledger is
    /// invalid, or the database is newer than this binary supports.
    pub fn open(path: &Path, options: &StoreOpenOptions) -> Result<Self, StorageError> {
        if options.app_version().trim().is_empty() {
            return Err(StorageError::InvalidOpenOption {
                field: "app_version",
            });
        }

        let mut connection = Connection::open(path).map_err(|_| StorageError::OpenFailed)?;
        let configuration = configure_writer(&connection)?;
        migration::apply_all(&mut connection, path, options)?;
        Ok(Self {
            connection,
            configuration,
        })
    }

    /// Returns the verified configuration for this writer connection.
    #[must_use]
    pub const fn configuration(&self) -> ConnectionConfiguration {
        self.configuration
    }

    /// Returns the schema version stored in singleton metadata.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if singleton metadata cannot be read safely.
    pub fn schema_version(&self) -> Result<u32, StorageError> {
        migration::metadata_schema_version(&self.connection)
    }

    /// Borrows the crate-private writer connection for a storage operation.
    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

/// The crate-owned query-only SQLite reader connection.
///
/// The raw SQLite connection remains private and is configured to reject writes.
pub struct SqliteReader {
    connection: Connection,
    configuration: ConnectionConfiguration,
}

impl SqliteReader {
    /// Opens and verifies a query-only SQLite reader connection.
    ///
    /// Migrations are intentionally not run here. A writer must finish them
    /// before reader workers start.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the database cannot be opened safely, a
    /// required reader setting cannot be verified, or the existing migration
    /// ledger is not compatible with this binary.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| StorageError::OpenFailed)?;
        let configuration = configure_reader(&connection)?;
        let schema_version = migration::verify_existing(&connection)?;
        if schema_version != migration::LATEST_SCHEMA_VERSION {
            return Err(StorageError::DatabaseRequiresMigration {
                database_version: schema_version,
                supported_version: migration::LATEST_SCHEMA_VERSION,
            });
        }
        Ok(Self {
            connection,
            configuration,
        })
    }

    /// Returns the verified configuration for this reader connection.
    #[must_use]
    pub const fn configuration(&self) -> ConnectionConfiguration {
        self.configuration
    }

    /// Returns the schema version stored in singleton metadata.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if singleton metadata cannot be read safely.
    pub fn schema_version(&self) -> Result<u32, StorageError> {
        migration::metadata_schema_version(&self.connection)
    }

    /// Borrows the crate-private reader connection for one short read transaction.
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Reapplies and verifies all mandatory writer settings after a connection reset or restore.
pub(crate) fn configure_writer(
    connection: &Connection,
) -> Result<ConnectionConfiguration, StorageError> {
    configure_busy_timeout(connection)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;\n             PRAGMA synchronous = FULL;\n             PRAGMA foreign_keys = ON;\n             PRAGMA trusted_schema = OFF;",
        )
        .map_err(|_| StorageError::ConnectionConfiguration {
            setting: ConnectionSetting::JournalMode,
        })?;

    verify_writer_configuration(connection)
}

/// Verifies all mandatory writer settings without changing the connection.
pub(crate) fn verify_writer_configuration(
    connection: &Connection,
) -> Result<ConnectionConfiguration, StorageError> {
    let journal_mode =
        query_text_pragma(connection, "journal_mode", ConnectionSetting::JournalMode)?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::ConnectionConfiguration {
            setting: ConnectionSetting::JournalMode,
        });
    }
    verify_integer_pragma(
        connection,
        "busy_timeout",
        BUSY_TIMEOUT_MILLISECONDS,
        ConnectionSetting::BusyTimeout,
    )?;
    verify_integer_pragma(
        connection,
        "synchronous",
        SQLITE_SYNCHRONOUS_FULL,
        ConnectionSetting::Synchronous,
    )?;
    verify_integer_pragma(
        connection,
        "foreign_keys",
        1,
        ConnectionSetting::ForeignKeys,
    )?;
    verify_integer_pragma(
        connection,
        "trusted_schema",
        0,
        ConnectionSetting::TrustedSchema,
    )?;

    Ok(ConnectionConfiguration {
        journal_mode: Some(JournalMode::Wal),
        synchronous: Some(SynchronousMode::Full),
        foreign_keys: true,
        trusted_schema: false,
        query_only: false,
        busy_timeout: BUSY_TIMEOUT,
    })
}

fn configure_reader(connection: &Connection) -> Result<ConnectionConfiguration, StorageError> {
    configure_busy_timeout(connection)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;\n             PRAGMA trusted_schema = OFF;\n             PRAGMA query_only = ON;",
        )
        .map_err(|_| StorageError::ConnectionConfiguration {
            setting: ConnectionSetting::QueryOnly,
        })?;
    verify_integer_pragma(
        connection,
        "foreign_keys",
        1,
        ConnectionSetting::ForeignKeys,
    )?;
    verify_integer_pragma(
        connection,
        "trusted_schema",
        0,
        ConnectionSetting::TrustedSchema,
    )?;
    verify_integer_pragma(connection, "query_only", 1, ConnectionSetting::QueryOnly)?;

    Ok(ConnectionConfiguration {
        journal_mode: None,
        synchronous: None,
        foreign_keys: true,
        trusted_schema: false,
        query_only: true,
        busy_timeout: BUSY_TIMEOUT,
    })
}

fn configure_busy_timeout(connection: &Connection) -> Result<(), StorageError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|_| StorageError::ConnectionConfiguration {
            setting: ConnectionSetting::BusyTimeout,
        })?;
    verify_integer_pragma(
        connection,
        "busy_timeout",
        BUSY_TIMEOUT_MILLISECONDS,
        ConnectionSetting::BusyTimeout,
    )
}

fn query_text_pragma(
    connection: &Connection,
    name: &'static str,
    setting: ConnectionSetting,
) -> Result<String, StorageError> {
    let query = format!("PRAGMA {name}");
    connection
        .query_row(&query, [], |row| row.get(0))
        .map_err(|_| StorageError::ConnectionConfiguration { setting })
}

fn verify_integer_pragma(
    connection: &Connection,
    name: &'static str,
    expected: i64,
    setting: ConnectionSetting,
) -> Result<(), StorageError> {
    let query = format!("PRAGMA {name}");
    let value: i64 = connection
        .query_row(&query, [], |row| row.get(0))
        .map_err(|_| StorageError::ConnectionConfiguration { setting })?;
    if value != expected {
        return Err(StorageError::ConnectionConfiguration { setting });
    }
    Ok(())
}
