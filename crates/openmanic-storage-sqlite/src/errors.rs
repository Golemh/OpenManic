//! Typed persistence failures that do not expose SQLite implementation types.

use core::fmt;

/// A failure while opening, configuring, validating, or migrating an OpenManic store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageError {
    /// A required open option did not contain a usable value.
    InvalidOpenOption {
        /// The invalid option name.
        field: &'static str,
    },
    /// SQLite could not open the requested database path.
    OpenFailed,
    /// SQLite could not apply or verify a required connection setting.
    ConnectionConfiguration {
        /// The connection setting that was not applied or verified.
        setting: ConnectionSetting,
    },
    /// A required schema query or mutation did not complete.
    DatabaseOperation {
        /// The named persistence operation that failed.
        operation: &'static str,
    },
    /// A database contains schema state but no migration ledger.
    MigrationLedgerMissing,
    /// A stored migration checksum differs from the immutable compiled migration source.
    MigrationChecksumMismatch {
        /// The migration version with the mismatched checksum.
        version: u32,
    },
    /// The database schema is newer than this binary can safely open.
    DatabaseNewerThanBinary {
        /// The highest schema version recorded by the database.
        database_version: u32,
        /// The newest schema version supported by this binary.
        supported_version: u32,
    },
    /// The required singleton metadata row is missing or malformed.
    MetadataInvalid,
    /// The caller supplied a store identity that does not match the existing database.
    StoreIdentityMismatch,
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOpenOption { field } => {
                write!(formatter, "invalid storage option: {field}")
            }
            Self::OpenFailed => formatter.write_str("could not open the local SQLite store"),
            Self::ConnectionConfiguration { setting } => {
                write!(formatter, "could not configure or verify SQLite {setting}")
            }
            Self::DatabaseOperation { operation } => {
                write!(formatter, "SQLite operation failed: {operation}")
            }
            Self::MigrationLedgerMissing => {
                formatter.write_str("SQLite schema exists without its required migration ledger")
            }
            Self::MigrationChecksumMismatch { version } => {
                write!(
                    formatter,
                    "SQLite migration {version} has an unexpected checksum"
                )
            }
            Self::DatabaseNewerThanBinary {
                database_version,
                supported_version,
            } => write!(
                formatter,
                "SQLite schema version {database_version} is newer than supported version {supported_version}"
            ),
            Self::MetadataInvalid => {
                formatter.write_str("SQLite store metadata is missing or malformed")
            }
            Self::StoreIdentityMismatch => {
                formatter.write_str("SQLite store identity does not match the requested store")
            }
        }
    }
}

impl std::error::Error for StorageError {}

/// A connection setting whose failure prevents safe storage operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionSetting {
    /// The WAL journal mode required by writer connections.
    JournalMode,
    /// The FULL durability mode required by writer connections.
    Synchronous,
    /// Foreign-key enforcement required by every connection.
    ForeignKeys,
    /// The disabled trusted-schema mode required by every connection.
    TrustedSchema,
    /// The read-only query mode required by reader connections.
    QueryOnly,
    /// The bounded SQLite busy timeout required by every connection.
    BusyTimeout,
}

impl fmt::Display for ConnectionSetting {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::JournalMode => "journal mode",
            Self::Synchronous => "synchronous mode",
            Self::ForeignKeys => "foreign-key enforcement",
            Self::TrustedSchema => "trusted-schema mode",
            Self::QueryOnly => "query-only mode",
            Self::BusyTimeout => "busy timeout",
        };
        formatter.write_str(name)
    }
}
