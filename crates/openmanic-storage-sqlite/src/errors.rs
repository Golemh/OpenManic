//! Typed persistence failures that do not expose SQLite implementation types.

use core::fmt;

/// A failure while opening, configuring, validating, or migrating an OpenManic store.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StorageError {
    /// A required open option did not contain a usable value.
    #[error("invalid storage option: {field}")]
    InvalidOpenOption {
        /// The invalid option name.
        field: &'static str,
    },
    /// SQLite could not open the requested database path.
    #[error("could not open the local SQLite store")]
    OpenFailed,
    /// SQLite could not apply or verify a required connection setting.
    #[error("could not configure or verify SQLite {setting}")]
    ConnectionConfiguration {
        /// The connection setting that was not applied or verified.
        setting: ConnectionSetting,
    },
    /// A required schema query or mutation did not complete.
    #[error("SQLite operation failed: {operation}")]
    DatabaseOperation {
        /// The named persistence operation that failed.
        operation: &'static str,
    },
    /// A database contains schema state but no migration ledger.
    #[error("SQLite schema exists without its required migration ledger")]
    MigrationLedgerMissing,
    /// A stored migration checksum differs from the immutable compiled migration source.
    #[error("SQLite migration {version} has an unexpected checksum")]
    MigrationChecksumMismatch {
        /// The migration version with the mismatched checksum.
        version: u32,
    },
    /// The database schema is newer than this binary can safely open.
    #[error(
        "SQLite schema version {database_version} is newer than supported version {supported_version}"
    )]
    DatabaseNewerThanBinary {
        /// The highest schema version recorded by the database.
        database_version: u32,
        /// The newest schema version supported by this binary.
        supported_version: u32,
    },
    /// The required singleton metadata row is missing or malformed.
    #[error("SQLite store metadata is missing or malformed")]
    MetadataInvalid,
    /// The caller supplied a store identity that does not match the existing database.
    #[error("SQLite store identity does not match the requested store")]
    StoreIdentityMismatch,
}

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

#[cfg(test)]
mod tests {
    use super::{ConnectionSetting, StorageError};

    #[test]
    fn storage_errors_preserve_messages_and_clone_equality() {
        let cases = [
            (
                StorageError::InvalidOpenOption { field: "store_id" },
                "invalid storage option: store_id",
            ),
            (
                StorageError::OpenFailed,
                "could not open the local SQLite store",
            ),
            (
                StorageError::ConnectionConfiguration {
                    setting: ConnectionSetting::JournalMode,
                },
                "could not configure or verify SQLite journal mode",
            ),
            (
                StorageError::DatabaseOperation {
                    operation: "apply migration",
                },
                "SQLite operation failed: apply migration",
            ),
            (
                StorageError::MigrationLedgerMissing,
                "SQLite schema exists without its required migration ledger",
            ),
            (
                StorageError::MigrationChecksumMismatch { version: 1 },
                "SQLite migration 1 has an unexpected checksum",
            ),
            (
                StorageError::DatabaseNewerThanBinary {
                    database_version: 2,
                    supported_version: 1,
                },
                "SQLite schema version 2 is newer than supported version 1",
            ),
            (
                StorageError::MetadataInvalid,
                "SQLite store metadata is missing or malformed",
            ),
            (
                StorageError::StoreIdentityMismatch,
                "SQLite store identity does not match the requested store",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
            assert_eq!(error, error.clone());
        }
    }
}
