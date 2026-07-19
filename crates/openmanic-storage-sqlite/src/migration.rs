//! Immutable SQL migration registry, validation, and recovery-safe execution.
//!
//! Initial-store creation does not need a backup because no user data exists.
//! Every later migration validates the existing ledger first, then creates and
//! verifies an online backup before its transaction begins. If that transaction
//! fails, the retained backup is restored before the error leaves this crate.

use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::backup::{
    IntegrityCheckFailure, VerifiedBackup, create_verified_backup, restore_verified_backup,
    verify_database_integrity,
};
use crate::{StorageError, StoreOpenOptions};

/// The newest migration version compiled into this storage crate.
pub const LATEST_SCHEMA_VERSION: u32 = 2;

const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial.sql");
const INITIAL_CHECKSUM: [u8; 8] = migration_checksum(INITIAL_SCHEMA);
const SCHEDULE_EXCEPTION_BOUNDARY_RESOLUTION_SCHEMA: &str =
    include_str!("../migrations/0002_schedule_exception_boundary_resolution.sql");
const SCHEDULE_EXCEPTION_BOUNDARY_RESOLUTION_CHECKSUM: [u8; 8] =
    migration_checksum(SCHEDULE_EXCEPTION_BOUNDARY_RESOLUTION_SCHEMA);
const MIGRATIONS: [Migration; 2] = [
    Migration {
        version: 1,
        source: INITIAL_SCHEMA,
        checksum: INITIAL_CHECKSUM,
    },
    Migration {
        version: 2,
        source: SCHEDULE_EXCEPTION_BOUNDARY_RESOLUTION_SCHEMA,
        checksum: SCHEDULE_EXCEPTION_BOUNDARY_RESOLUTION_CHECKSUM,
    },
];

#[derive(Clone, Copy)]
struct Migration {
    version: u32,
    source: &'static str,
    checksum: [u8; 8],
}

pub(crate) fn apply_all(
    connection: &mut Connection,
    database_path: &Path,
    options: &StoreOpenOptions,
) -> Result<(), StorageError> {
    if table_exists(connection, "schema_migration")? {
        migrate_existing(connection, database_path, options)?;
        update_last_opened_version(connection, options)
    } else if has_user_tables(connection)? {
        Err(StorageError::MigrationLedgerMissing)
    } else {
        apply_initial_schema(connection, options)?;
        migrate_existing(connection, database_path, options)?;
        update_last_opened_version(connection, options)
    }
}

pub(crate) fn verify_existing(connection: &Connection) -> Result<u32, StorageError> {
    verify_existing_with(connection, &MIGRATIONS)
}

fn verify_existing_with(
    connection: &Connection,
    migrations: &[Migration],
) -> Result<u32, StorageError> {
    if !table_exists(connection, "schema_migration")? {
        return Err(StorageError::MigrationLedgerMissing);
    }

    let supported_version = latest_version(migrations)?;
    let user_version = read_pragma_user_version(connection)?;
    if user_version > supported_version {
        return Err(StorageError::DatabaseNewerThanBinary {
            database_version: user_version,
            supported_version,
        });
    }

    let mut statement = connection
        .prepare("SELECT version, checksum FROM schema_migration ORDER BY version")
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "read migration ledger",
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "read migration ledger",
        })?;
    let mut applied_version = 0_u32;
    for row in rows {
        let (version, checksum) = row.map_err(|_| StorageError::DatabaseOperation {
            operation: "read migration ledger",
        })?;
        let version = u32::try_from(version).map_err(|_| StorageError::MigrationLedgerMissing)?;
        if version > supported_version {
            return Err(StorageError::DatabaseNewerThanBinary {
                database_version: version,
                supported_version,
            });
        }
        let expected_version = applied_version
            .checked_add(1)
            .ok_or(StorageError::MigrationLedgerMissing)?;
        if version != expected_version {
            return Err(StorageError::MigrationLedgerMissing);
        }
        let Some(compiled) = migrations
            .iter()
            .find(|migration| migration.version == version)
        else {
            return Err(StorageError::MigrationLedgerMissing);
        };
        if checksum.as_slice() != compiled.checksum {
            return Err(StorageError::MigrationChecksumMismatch { version });
        }
        applied_version = version;
    }
    if applied_version == 0 {
        return Err(StorageError::MigrationLedgerMissing);
    }

    let metadata_version = metadata_schema_version(connection)?;
    if metadata_version > supported_version {
        return Err(StorageError::DatabaseNewerThanBinary {
            database_version: metadata_version,
            supported_version,
        });
    }
    if metadata_version != applied_version || user_version != applied_version {
        return Err(StorageError::MetadataInvalid);
    }
    Ok(applied_version)
}

fn latest_version(migrations: &[Migration]) -> Result<u32, StorageError> {
    migrations
        .last()
        .map(|migration| migration.version)
        .ok_or(StorageError::MigrationLedgerMissing)
}

fn migrate_existing(
    connection: &mut Connection,
    database_path: &Path,
    options: &StoreOpenOptions,
) -> Result<(), StorageError> {
    migrate_existing_with_integrity(
        connection,
        database_path,
        options,
        &MIGRATIONS,
        apply_migration_sql,
        verify_post_migration_integrity,
    )
}

#[cfg(test)]
fn migrate_existing_with<F>(
    connection: &mut Connection,
    database_path: &Path,
    options: &StoreOpenOptions,
    migrations: &[Migration],
    apply: F,
) -> Result<(), StorageError>
where
    F: FnMut(&Transaction<'_>, Migration, &VerifiedBackup) -> Result<(), StorageError>,
{
    migrate_existing_with_integrity(
        connection,
        database_path,
        options,
        migrations,
        apply,
        verify_post_migration_integrity,
    )
}

fn migrate_existing_with_integrity<F, I>(
    connection: &mut Connection,
    database_path: &Path,
    options: &StoreOpenOptions,
    migrations: &[Migration],
    apply: F,
    verify_integrity: I,
) -> Result<(), StorageError>
where
    F: FnMut(&Transaction<'_>, Migration, &VerifiedBackup) -> Result<(), StorageError>,
    I: FnMut(&Transaction<'_>, Migration) -> Result<(), StorageError>,
{
    // Ledger, checksum, schema-version, and store-identity failures must not
    // create a backup or begin migration work.
    let current_version = verify_existing_with(connection, migrations)?;
    verify_store_identity(connection, options)?;
    apply_pending_migrations_with(
        connection,
        database_path,
        options,
        current_version,
        migrations,
        apply,
        verify_integrity,
    )
}

fn apply_pending_migrations_with<F, I>(
    connection: &mut Connection,
    database_path: &Path,
    options: &StoreOpenOptions,
    current_version: u32,
    migrations: &[Migration],
    mut apply: F,
    mut verify_integrity: I,
) -> Result<(), StorageError>
where
    F: FnMut(&Transaction<'_>, Migration, &VerifiedBackup) -> Result<(), StorageError>,
    I: FnMut(&Transaction<'_>, Migration) -> Result<(), StorageError>,
{
    let mut applied_version = current_version;
    for migration in migrations {
        if migration.version <= applied_version {
            continue;
        }

        let backup = create_verified_backup(
            connection,
            database_path,
            applied_version,
            migration.version,
        )?;
        let result = apply_migration_transactionally(
            connection,
            options,
            *migration,
            &backup,
            &mut apply,
            &mut verify_integrity,
        );
        if let Err(error) = result {
            restore_verified_backup(connection, &backup)?;
            return Err(migration_error_after_recovery(migration.version, error));
        }
        applied_version = migration.version;
    }
    Ok(())
}

fn apply_migration_transactionally<F, I>(
    connection: &mut Connection,
    options: &StoreOpenOptions,
    migration: Migration,
    backup: &VerifiedBackup,
    apply: &mut F,
    verify_integrity: &mut I,
) -> Result<(), StorageError>
where
    F: FnMut(&Transaction<'_>, Migration, &VerifiedBackup) -> Result<(), StorageError>,
    I: FnMut(&Transaction<'_>, Migration) -> Result<(), StorageError>,
{
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "begin migration",
        })?;
    apply(&transaction, migration, backup)?;
    record_migration(&transaction, options, migration)?;
    update_schema_version(&transaction, migration.version)?;
    verify_integrity(&transaction, migration)?;
    transaction
        .commit()
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "commit migration",
        })
}

fn verify_post_migration_integrity(
    transaction: &Transaction<'_>,
    migration: Migration,
) -> Result<(), StorageError> {
    verify_database_integrity(transaction).map_err(|failure| match failure {
        IntegrityCheckFailure::QuickCheck => StorageError::MigrationQuickCheckFailed {
            version: migration.version,
        },
        IntegrityCheckFailure::ForeignKeyCheck => StorageError::MigrationForeignKeyCheckFailed {
            version: migration.version,
        },
    })
}

fn migration_error_after_recovery(version: u32, error: StorageError) -> StorageError {
    match error {
        StorageError::MigrationQuickCheckFailed { .. }
        | StorageError::MigrationForeignKeyCheckFailed { .. } => error,
        _ => StorageError::MigrationFailed { version },
    }
}

fn apply_migration_sql(
    transaction: &Transaction<'_>,
    migration: Migration,
    _: &VerifiedBackup,
) -> Result<(), StorageError> {
    transaction
        .execute_batch(migration.source)
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "apply migration",
        })
}

pub(crate) fn metadata_schema_version(connection: &Connection) -> Result<u32, StorageError> {
    let version: i64 = connection
        .query_row(
            "SELECT schema_version FROM store_metadata WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| StorageError::MetadataInvalid)?;
    u32::try_from(version).map_err(|_| StorageError::MetadataInvalid)
}

fn record_migration(
    transaction: &Transaction<'_>,
    options: &StoreOpenOptions,
    migration: Migration,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO schema_migration(version, checksum, applied_utc_us, app_version)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                i64::from(migration.version),
                migration.checksum.as_slice(),
                options.opened_utc_us(),
                options.app_version(),
            ],
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "record migration",
        })?;
    Ok(())
}

fn update_schema_version(transaction: &Transaction<'_>, version: u32) -> Result<(), StorageError> {
    let changed = transaction
        .execute(
            "UPDATE store_metadata SET schema_version = ?1 WHERE singleton_id = 1",
            [i64::from(version)],
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "update schema metadata",
        })?;
    if changed != 1 {
        return Err(StorageError::MetadataInvalid);
    }
    transaction
        .pragma_update(None, "user_version", i64::from(version))
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "set SQLite user version",
        })?;
    Ok(())
}

fn apply_initial_schema(
    connection: &mut Connection,
    options: &StoreOpenOptions,
) -> Result<(), StorageError> {
    let user_version = read_pragma_user_version(connection)?;
    if user_version > LATEST_SCHEMA_VERSION {
        return Err(StorageError::DatabaseNewerThanBinary {
            database_version: user_version,
            supported_version: LATEST_SCHEMA_VERSION,
        });
    }
    if user_version != 0 {
        return Err(StorageError::MigrationLedgerMissing);
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "begin initial migration",
        })?;
    transaction
        .execute_batch(INITIAL_SCHEMA)
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "apply initial migration",
        })?;
    record_migration(&transaction, options, MIGRATIONS[0])?;
    transaction
        .execute(
            "INSERT INTO store_metadata(
                 singleton_id, store_id, data_revision, schema_version,
                 created_utc_us, last_opened_app_version, last_clean_shutdown_utc_us
             ) VALUES (1, ?1, 0, ?2, ?3, ?4, NULL)",
            params![
                options.store_id().as_slice(),
                i64::from(MIGRATIONS[0].version),
                options.opened_utc_us(),
                options.app_version(),
            ],
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "initialize store metadata",
        })?;
    transaction
        .pragma_update(None, "user_version", i64::from(MIGRATIONS[0].version))
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "set SQLite user version",
        })?;
    transaction
        .commit()
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "commit initial migration",
        })
}

fn verify_store_identity(
    connection: &Connection,
    options: &StoreOpenOptions,
) -> Result<(), StorageError> {
    let stored: Vec<u8> = connection
        .query_row(
            "SELECT store_id FROM store_metadata WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| StorageError::MetadataInvalid)?;
    if stored.as_slice() != options.store_id() {
        return Err(StorageError::StoreIdentityMismatch);
    }
    Ok(())
}

fn update_last_opened_version(
    connection: &Connection,
    options: &StoreOpenOptions,
) -> Result<(), StorageError> {
    let changed = connection
        .execute(
            "UPDATE store_metadata
             SET last_opened_app_version = ?1
             WHERE singleton_id = 1",
            [options.app_version()],
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "update store open metadata",
        })?;
    if changed != 1 {
        return Err(StorageError::MetadataInvalid);
    }
    Ok(())
}

fn table_exists(connection: &Connection, table: &'static str) -> Result<bool, StorageError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get(0),
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "inspect SQLite schema",
        })
}

fn has_user_tables(connection: &Connection) -> Result<bool, StorageError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM sqlite_master
                WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "inspect SQLite schema",
        })
}

fn read_pragma_user_version(connection: &Connection) -> Result<u32, StorageError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "read SQLite user version",
        })?;
    u32::try_from(version).map_err(|_| StorageError::MetadataInvalid)
}

const fn migration_checksum(source: &str) -> [u8; 8] {
    let bytes = source.as_bytes();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use rusqlite::Connection;

    use crate::connection;
    use crate::{
        ConnectionConfiguration, JournalMode, SqliteWriter, StorageError, StoreOpenOptions,
        SynchronousMode,
    };

    use super::{
        MIGRATIONS, Migration, apply_migration_sql, create_verified_backup, migrate_existing_with,
        migrate_existing_with_integrity, restore_verified_backup,
    };

    static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    const TEST_MIGRATION_SOURCE: &str =
        "CREATE TABLE migration_safety_probe (id INTEGER PRIMARY KEY) STRICT;";
    const FOREIGN_KEY_FAILURE_MIGRATION_SOURCE: &str = "INSERT INTO application(
        public_id, display_name, category_id, exclusion_policy, first_seen_utc_us, last_seen_utc_us
    ) VALUES (X'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'invalid migration reference', 999, 0, 0, 0);";

    struct TemporaryDatabase {
        path: PathBuf,
    }

    impl TemporaryDatabase {
        fn new(case_name: &str) -> Self {
            let identifier = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            let filename = format!(
                "openmanic-om151-{case_name}-{}-{identifier}.sqlite3",
                std::process::id()
            );
            let path = std::env::temp_dir().join(filename);
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(sidecar_path(&path, "-shm"));
            let _ = fs::remove_file(sidecar_path(&path, "-wal"));
            remove_retained_backups(&path);
            Self { path }
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
            remove_retained_backups(&self.path);
        }
    }

    fn sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
        let mut sidecar = OsString::from(database_path.as_os_str());
        sidecar.push(suffix);
        PathBuf::from(sidecar)
    }

    fn remove_retained_backups(database_path: &Path) {
        let Some(directory) = database_path.parent() else {
            return;
        };
        let Some(filename) = database_path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let prefix = format!("{filename}.pre-migration-");
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn open_options(store_byte: u8) -> StoreOpenOptions {
        StoreOpenOptions::new([store_byte; 16], 1_725_000_000_000_000, "0.1.0-test")
    }

    fn initialized_connection(
        database: &TemporaryDatabase,
        options: &StoreOpenOptions,
    ) -> Connection {
        let mut connection =
            Connection::open(database.path()).expect("the isolated SQLite fixture should open");
        connection
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
            .expect("the isolated SQLite fixture should configure WAL and foreign keys");
        super::apply_initial_schema(&mut connection, options)
            .expect("the isolated SQLite fixture should apply migration 0001");
        connection
    }

    fn test_migrations() -> [Migration; 2] {
        [
            MIGRATIONS[0],
            Migration {
                version: 2,
                source: TEST_MIGRATION_SOURCE,
                checksum: super::migration_checksum(TEST_MIGRATION_SOURCE),
            },
        ]
    }

    fn foreign_key_failure_migrations() -> [Migration; 2] {
        [
            MIGRATIONS[0],
            Migration {
                version: 2,
                source: FOREIGN_KEY_FAILURE_MIGRATION_SOURCE,
                checksum: super::migration_checksum(FOREIGN_KEY_FAILURE_MIGRATION_SOURCE),
            },
        ]
    }

    fn assert_writer_configuration(configuration: ConnectionConfiguration) {
        assert_eq!(configuration.journal_mode(), Some(JournalMode::Wal));
        assert_eq!(configuration.synchronous(), Some(SynchronousMode::Full));
        assert!(configuration.foreign_keys());
        assert!(!configuration.trusted_schema());
        assert!(!configuration.query_only());
        assert_eq!(configuration.busy_timeout(), Duration::from_secs(5));
    }

    fn retained_backup_count(database_path: &Path) -> usize {
        let Some(directory) = database_path.parent() else {
            return 0;
        };
        let Some(filename) = database_path.file_name().and_then(|name| name.to_str()) else {
            return 0;
        };
        let prefix = format!("{filename}.pre-migration-");
        fs::read_dir(directory)
            .expect("the temporary directory should remain readable")
            .flatten()
            .filter(|entry| {
                entry.file_name().to_string_lossy().starts_with(&prefix)
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "sqlite3")
            })
            .count()
    }

    #[test]
    fn verified_backup_exists_before_an_injected_migration_failure() {
        let database = TemporaryDatabase::new("backup-before-failure");
        let options = open_options(1);
        let mut connection = initialized_connection(&database, &options);
        let migrations = test_migrations();
        let mut migration_started = false;
        let mut verified_backup_path = None;

        let error = migrate_existing_with(
            &mut connection,
            database.path(),
            &options,
            &migrations,
            |transaction, migration, backup| {
                migration_started = true;
                verified_backup_path = Some(backup.path().to_path_buf());
                let revision: i64 = Connection::open(backup.path())
                    .expect("the verified backup should be independently readable")
                    .query_row(
                        "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
                        [],
                        |row| row.get(0),
                    )
                    .expect("the verified backup should contain initial metadata");
                assert_eq!(revision, 0);
                transaction
                    .execute(
                        "UPDATE store_metadata SET data_revision = 99 WHERE singleton_id = 1",
                        [],
                    )
                    .expect("the injected migration can modify its transaction before failing");
                assert_eq!(migration.version, 2);
                Err(StorageError::DatabaseOperation {
                    operation: "forced migration failure",
                })
            },
        )
        .expect_err("the injected migration failure must be reported");

        assert_eq!(error, StorageError::MigrationFailed { version: 2 });
        assert!(migration_started);
        let backup_path =
            verified_backup_path.expect("migration must observe a verified backup first");
        assert!(backup_path.is_file());
        assert!(!sidecar_path(&backup_path, "-shm").exists());
        assert!(!sidecar_path(&backup_path, "-wal").exists());
        assert_eq!(retained_backup_count(database.path()), 1);
        assert_writer_configuration(
            connection::verify_writer_configuration(&connection)
                .expect("the restored current connection should be configured as a writer"),
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("the original database must remain usable after restoration"),
            0
        );
        let reopened = Connection::open(database.path())
            .expect("the restored database must remain reopenable");
        assert_eq!(
            reopened
                .query_row(
                    "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("the reopened original database must retain its prior state"),
            0
        );
    }

    #[test]
    fn retained_backup_can_restore_mutated_original_database() {
        let database = TemporaryDatabase::new("backup-restore");
        let options = open_options(2);
        let mut connection = initialized_connection(&database, &options);
        let backup = create_verified_backup(&connection, database.path(), 1, 2)
            .expect("the initial database should produce a verified online backup");
        let retained_path = backup.path().to_path_buf();

        connection
            .execute(
                "UPDATE store_metadata SET data_revision = 77 WHERE singleton_id = 1",
                [],
            )
            .expect("the source mutation should persist before restore");
        restore_verified_backup(&mut connection, &backup)
            .expect("the verified backup should restore the original state");

        assert!(retained_path.is_file());
        assert_writer_configuration(
            connection::verify_writer_configuration(&connection)
                .expect("the restored current connection should be configured as a writer"),
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT data_revision FROM store_metadata WHERE singleton_id = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("the restored database should remain queryable"),
            0
        );
        let reopened = SqliteWriter::open(database.path(), &options)
            .expect("the restored database should reopen as a configured writer");
        assert_writer_configuration(reopened.configuration());
    }

    #[test]
    fn successful_post_initial_migration_passes_integrity_before_commit() {
        let database = TemporaryDatabase::new("post-migration-success");
        let options = open_options(6);
        let mut connection = initialized_connection(&database, &options);
        let migrations = test_migrations();

        migrate_existing_with(
            &mut connection,
            database.path(),
            &options,
            &migrations,
            apply_migration_sql,
        )
        .expect("a valid post-initial migration should pass both integrity checks");

        assert_eq!(super::verify_existing_with(&connection, &migrations), Ok(2));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM schema_migration", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("the successful migration ledger should remain readable"),
            2
        );
    }

    #[test]
    fn post_migration_foreign_key_failure_rolls_back_and_restores_original_database() {
        let database = TemporaryDatabase::new("post-migration-foreign-key-failure");
        let options = open_options(7);
        let mut connection = initialized_connection(&database, &options);
        let migrations = foreign_key_failure_migrations();
        connection
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("the fixture can model a migration containing invalid foreign-key data");

        let error = migrate_existing_with(
            &mut connection,
            database.path(),
            &options,
            &migrations,
            apply_migration_sql,
        )
        .expect_err("the post-migration foreign-key check must reject the transaction");

        assert_eq!(
            error,
            StorageError::MigrationForeignKeyCheckFailed { version: 2 }
        );
        assert_eq!(super::verify_existing_with(&connection, &MIGRATIONS), Ok(1));
        assert_writer_configuration(
            connection::verify_writer_configuration(&connection)
                .expect("the restored current connection should be configured as a writer"),
        );
        let reopened = SqliteWriter::open(database.path(), &options)
            .expect("the restored original database should reopen as a writer");
        assert_eq!(reopened.schema_version(), Ok(2));
        assert_writer_configuration(reopened.configuration());
    }

    #[test]
    fn integrity_checkpoint_runs_after_metadata_updates_and_before_commit() {
        let database = TemporaryDatabase::new("integrity-ordering");
        let options = open_options(8);
        let mut connection = initialized_connection(&database, &options);
        let migrations = test_migrations();
        let mut integrity_checkpoint_observed = false;

        let error = migrate_existing_with_integrity(
            &mut connection,
            database.path(),
            &options,
            &migrations,
            apply_migration_sql,
            |transaction, migration| {
                assert_eq!(migration.version, 2);
                assert_eq!(
                    transaction
                        .query_row(
                            "SELECT COUNT(*) FROM schema_migration WHERE version = 2",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .expect("the integrity checkpoint should observe its ledger row"),
                    1
                );
                assert_eq!(
                    transaction
                        .query_row(
                            "SELECT schema_version FROM store_metadata WHERE singleton_id = 1",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .expect("the integrity checkpoint should observe updated metadata"),
                    2
                );
                assert_eq!(
                    transaction
                        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                        .expect("the integrity checkpoint should observe the updated user version"),
                    2
                );
                integrity_checkpoint_observed = true;
                Err(StorageError::MigrationQuickCheckFailed { version: 2 })
            },
        )
        .expect_err("an integrity failure must reject the migration before commit");

        assert_eq!(
            error,
            StorageError::MigrationQuickCheckFailed { version: 2 }
        );
        assert!(integrity_checkpoint_observed);
        assert_eq!(super::verify_existing_with(&connection, &MIGRATIONS), Ok(1));
        assert_writer_configuration(
            connection::verify_writer_configuration(&connection)
                .expect("the restored current connection should be configured as a writer"),
        );
    }

    #[test]
    fn newer_database_is_refused_without_creating_a_backup_or_starting_migration() {
        let database = TemporaryDatabase::new("newer-no-backup");
        let options = open_options(3);
        let mut connection = initialized_connection(&database, &options);
        connection
            .execute(
                "UPDATE store_metadata SET schema_version = 3 WHERE singleton_id = 1",
                [],
            )
            .expect("the fixture can represent a newer database");
        connection
            .pragma_update(None, "user_version", 3_i64)
            .expect("the fixture can represent a newer SQLite user version");
        let mut migration_started = false;

        let error = migrate_existing_with(
            &mut connection,
            database.path(),
            &options,
            &test_migrations(),
            |_, _, _| {
                migration_started = true;
                Ok(())
            },
        )
        .expect_err("a newer database must be refused before migration work");

        assert_eq!(
            error,
            StorageError::DatabaseNewerThanBinary {
                database_version: 3,
                supported_version: 2,
            }
        );
        assert!(!migration_started);
        assert_eq!(retained_backup_count(database.path()), 0);
    }

    #[test]
    fn checksum_mismatch_is_refused_without_creating_a_backup_or_starting_migration() {
        let database = TemporaryDatabase::new("checksum-no-backup");
        let options = open_options(4);
        let mut connection = initialized_connection(&database, &options);
        connection
            .execute(
                "UPDATE schema_migration SET checksum = ?1 WHERE version = 1",
                [vec![0_u8; 8]],
            )
            .expect("the fixture can represent a ledger checksum mismatch");
        let mut migration_started = false;

        let error = migrate_existing_with(
            &mut connection,
            database.path(),
            &options,
            &test_migrations(),
            |_, _, _| {
                migration_started = true;
                Ok(())
            },
        )
        .expect_err("a checksum mismatch must be refused before migration work");

        assert_eq!(
            error,
            StorageError::MigrationChecksumMismatch { version: 1 }
        );
        assert!(!migration_started);
        assert_eq!(retained_backup_count(database.path()), 0);
    }

    #[test]
    fn backup_foreign_key_verification_failure_prevents_migration() {
        let database = TemporaryDatabase::new("verification-no-migration");
        let options = open_options(5);
        let mut connection = initialized_connection(&database, &options);
        connection
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("the fixture can create intentionally invalid foreign-key data");
        connection
            .execute(
                "INSERT INTO application(
                     public_id, display_name, category_id, exclusion_policy,
                     first_seen_utc_us, last_seen_utc_us
                 ) VALUES (?1, 'invalid reference', 999, 0, 0, 0)",
                [vec![5_u8; 16]],
            )
            .expect("foreign-key enforcement is disabled only for this corrupt fixture");
        let mut migration_started = false;

        let error = migrate_existing_with(
            &mut connection,
            database.path(),
            &options,
            &test_migrations(),
            |_, _, _| {
                migration_started = true;
                Ok(())
            },
        )
        .expect_err("an unverifiable backup must prevent migration");

        assert_eq!(error, StorageError::BackupForeignKeyCheckFailed);
        assert!(!migration_started);
        assert_eq!(retained_backup_count(database.path()), 1);
    }
}
