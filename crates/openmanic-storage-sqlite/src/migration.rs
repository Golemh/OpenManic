//! Immutable SQL migration registry and ledger verification.

use rusqlite::{Connection, TransactionBehavior, params};

use crate::{StorageError, StoreOpenOptions};

/// The newest migration version compiled into this storage crate.
pub const LATEST_SCHEMA_VERSION: u32 = 1;

const INITIAL_SCHEMA: &str = include_str!("../migrations/0001_initial.sql");
const INITIAL_CHECKSUM: [u8; 8] = migration_checksum(INITIAL_SCHEMA);

pub(crate) fn apply_all(
    connection: &mut Connection,
    options: &StoreOpenOptions,
) -> Result<(), StorageError> {
    if table_exists(connection, "schema_migration")? {
        verify_existing(connection)?;
        verify_store_identity(connection, options)?;
        update_last_opened_version(connection, options)
    } else if has_user_tables(connection)? {
        Err(StorageError::MigrationLedgerMissing)
    } else {
        apply_initial_schema(connection, options)
    }
}

pub(crate) fn verify_existing(connection: &Connection) -> Result<(), StorageError> {
    if !table_exists(connection, "schema_migration")? {
        return Err(StorageError::MigrationLedgerMissing);
    }

    let user_version = read_pragma_user_version(connection)?;
    if user_version > LATEST_SCHEMA_VERSION {
        return Err(StorageError::DatabaseNewerThanBinary {
            database_version: user_version,
            supported_version: LATEST_SCHEMA_VERSION,
        });
    }

    let mut found_initial = false;
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
    for row in rows {
        let (version, checksum) = row.map_err(|_| StorageError::DatabaseOperation {
            operation: "read migration ledger",
        })?;
        let version = u32::try_from(version).map_err(|_| StorageError::MigrationLedgerMissing)?;
        if version > LATEST_SCHEMA_VERSION {
            return Err(StorageError::DatabaseNewerThanBinary {
                database_version: version,
                supported_version: LATEST_SCHEMA_VERSION,
            });
        }
        if version != 1 {
            return Err(StorageError::MigrationLedgerMissing);
        }
        if checksum.as_slice() != INITIAL_CHECKSUM {
            return Err(StorageError::MigrationChecksumMismatch { version });
        }
        found_initial = true;
    }
    if !found_initial {
        return Err(StorageError::MigrationLedgerMissing);
    }

    let schema_version = metadata_schema_version(connection)?;
    if schema_version > LATEST_SCHEMA_VERSION {
        return Err(StorageError::DatabaseNewerThanBinary {
            database_version: schema_version,
            supported_version: LATEST_SCHEMA_VERSION,
        });
    }
    if schema_version != LATEST_SCHEMA_VERSION || user_version != schema_version {
        return Err(StorageError::MetadataInvalid);
    }
    Ok(())
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
    transaction
        .execute(
            "INSERT INTO schema_migration(version, checksum, applied_utc_us, app_version)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                i64::from(LATEST_SCHEMA_VERSION),
                INITIAL_CHECKSUM.as_slice(),
                options.opened_utc_us(),
                options.app_version(),
            ],
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "record initial migration",
        })?;
    transaction
        .execute(
            "INSERT INTO store_metadata(
                 singleton_id, store_id, data_revision, schema_version,
                 created_utc_us, last_opened_app_version, last_clean_shutdown_utc_us
             ) VALUES (1, ?1, 0, ?2, ?3, ?4, NULL)",
            params![
                options.store_id().as_slice(),
                i64::from(LATEST_SCHEMA_VERSION),
                options.opened_utc_us(),
                options.app_version(),
            ],
        )
        .map_err(|_| StorageError::DatabaseOperation {
            operation: "initialize store metadata",
        })?;
    transaction
        .pragma_update(None, "user_version", i64::from(LATEST_SCHEMA_VERSION))
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
