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
    /// SQLite could not obtain a required lock within the configured bounded wait.
    #[error("SQLite remained busy during {operation}")]
    Busy {
        /// The operation that exhausted its bounded busy wait.
        operation: &'static str,
    },
    /// An authoritative mutation would exceed the supported data-revision range.
    #[error("SQLite data revision cannot advance further")]
    RevisionOverflow,
    /// A tracker run required by an activity mutation has not been registered.
    #[error("SQLite tracker run is not registered")]
    TrackerRunMissing,
    /// A category targeted by a catalog mutation is no longer present.
    #[error("SQLite category is not registered")]
    CategoryMissing,
    /// An application targeted by a catalog mutation is no longer present.
    #[error("SQLite application is not registered")]
    ApplicationMissing,
    /// A recovered checkpoint boundary predates its last trusted confirmation.
    #[error("SQLite recovery boundary predates the last trusted checkpoint")]
    RecoveryBoundaryBeforeCheckpoint,
    /// The supplied post-recovery tracking intent cannot safely start the next tracker run.
    #[error(
        "SQLite recovery requires a checkpoint-only intent for its registered next tracker run"
    )]
    RecoveryIntentInvalid,
    /// A stored SQLite value is incompatible with the frozen OpenManic schema.
    #[error("SQLite contains an invalid stored {field}")]
    InvalidStoredValue {
        /// The semantic field whose stored representation was invalid.
        field: &'static str,
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
    /// A reader encountered a database that requires a writer to apply known migrations first.
    #[error(
        "SQLite schema version {database_version} requires migration before this binary can read version {supported_version}"
    )]
    DatabaseRequiresMigration {
        /// The highest schema version currently recorded by the database.
        database_version: u32,
        /// The newest schema version supported by this binary.
        supported_version: u32,
    },
    /// No recoverable path could be reserved for a pre-migration backup.
    #[error("could not reserve a retained SQLite pre-migration backup path")]
    BackupPathUnavailable,
    /// SQLite could not create the required online pre-migration backup.
    #[error("could not create the required SQLite online pre-migration backup")]
    BackupCreationFailed,
    /// SQLite could not open a created backup for independent verification.
    #[error("could not open the SQLite pre-migration backup for verification")]
    BackupVerificationOpenFailed,
    /// SQLite's quick check rejected the pre-migration backup.
    #[error("SQLite quick check rejected the pre-migration backup")]
    BackupQuickCheckFailed,
    /// SQLite's foreign-key check rejected the pre-migration backup.
    #[error("SQLite foreign-key check rejected the pre-migration backup")]
    BackupForeignKeyCheckFailed,
    /// The restored database failed SQLite's quick check after writer settings were reapplied.
    #[error("SQLite quick check rejected the restored database")]
    RestoredDatabaseQuickCheckFailed,
    /// The restored database failed SQLite's foreign-key check after writer settings were reapplied.
    #[error("SQLite foreign-key check rejected the restored database")]
    RestoredDatabaseForeignKeyCheckFailed,
    /// A post-migration quick check failed before SQLite accepted the migration transaction.
    #[error("SQLite quick check rejected migration {version} before commit")]
    MigrationQuickCheckFailed {
        /// The migration version whose transaction was rejected.
        version: u32,
    },
    /// A post-migration foreign-key check failed before SQLite accepted the migration transaction.
    #[error("SQLite foreign-key check rejected migration {version} before commit")]
    MigrationForeignKeyCheckFailed {
        /// The migration version whose transaction was rejected.
        version: u32,
    },
    /// A post-initial migration failed after its verified backup was retained and restored.
    #[error("SQLite migration {version} failed; the retained pre-migration backup was restored")]
    MigrationFailed {
        /// The migration version that failed.
        version: u32,
    },
    /// The backup API could not restore a retained recovery snapshot after migration failure.
    #[error("SQLite migration failed and the retained pre-migration backup could not be restored")]
    BackupRestoreFailed,
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
    fn established_storage_errors_preserve_messages_and_clone_equality() {
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
                StorageError::Busy {
                    operation: "persist tracking",
                },
                "SQLite remained busy during persist tracking",
            ),
            (
                StorageError::RevisionOverflow,
                "SQLite data revision cannot advance further",
            ),
            (
                StorageError::TrackerRunMissing,
                "SQLite tracker run is not registered",
            ),
            (
                StorageError::RecoveryBoundaryBeforeCheckpoint,
                "SQLite recovery boundary predates the last trusted checkpoint",
            ),
            (
                StorageError::RecoveryIntentInvalid,
                "SQLite recovery requires a checkpoint-only intent for its registered next tracker run",
            ),
            (
                StorageError::InvalidStoredValue {
                    field: "activity state",
                },
                "SQLite contains an invalid stored activity state",
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

        assert_error_messages(&cases);
    }

    #[test]
    fn migration_safety_errors_preserve_messages_and_clone_equality() {
        let cases = [
            (
                StorageError::DatabaseRequiresMigration {
                    database_version: 1,
                    supported_version: 2,
                },
                "SQLite schema version 1 requires migration before this binary can read version 2",
            ),
            (
                StorageError::BackupPathUnavailable,
                "could not reserve a retained SQLite pre-migration backup path",
            ),
            (
                StorageError::BackupCreationFailed,
                "could not create the required SQLite online pre-migration backup",
            ),
            (
                StorageError::BackupVerificationOpenFailed,
                "could not open the SQLite pre-migration backup for verification",
            ),
            (
                StorageError::BackupQuickCheckFailed,
                "SQLite quick check rejected the pre-migration backup",
            ),
            (
                StorageError::BackupForeignKeyCheckFailed,
                "SQLite foreign-key check rejected the pre-migration backup",
            ),
            (
                StorageError::RestoredDatabaseQuickCheckFailed,
                "SQLite quick check rejected the restored database",
            ),
            (
                StorageError::RestoredDatabaseForeignKeyCheckFailed,
                "SQLite foreign-key check rejected the restored database",
            ),
            (
                StorageError::MigrationQuickCheckFailed { version: 2 },
                "SQLite quick check rejected migration 2 before commit",
            ),
            (
                StorageError::MigrationForeignKeyCheckFailed { version: 2 },
                "SQLite foreign-key check rejected migration 2 before commit",
            ),
            (
                StorageError::MigrationFailed { version: 2 },
                "SQLite migration 2 failed; the retained pre-migration backup was restored",
            ),
            (
                StorageError::BackupRestoreFailed,
                "SQLite migration failed and the retained pre-migration backup could not be restored",
            ),
        ];

        assert_error_messages(&cases);
    }

    fn assert_error_messages(cases: &[(StorageError, &str)]) {
        for (error, expected) in cases {
            assert_eq!(error.to_string(), *expected);
            let cloned = error.clone();
            assert_eq!(error, &cloned);
        }
    }
}
