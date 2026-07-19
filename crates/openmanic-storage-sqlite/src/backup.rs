//! Verified online-backup and restore operations for migration recovery.
//!
//! The guard deliberately uses SQLite's online backup API instead of copying a
//! live database file. WAL databases consist of the database and its sidecars,
//! so copying only the main file would not form a recoverable snapshot.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, OpenFlags};

use crate::StorageError;
use crate::connection;

/// The database check that rejected a backup, restored store, or migration transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntegrityCheckFailure {
    /// SQLite's quick check could not establish basic structural consistency.
    QuickCheck,
    /// SQLite found a foreign-key violation.
    ForeignKeyCheck,
}

/// A backup that passed the recovery checks required before a migration starts.
///
/// The path remains private to the storage implementation until a future
/// user-directed recovery flow owns its presentation and confirmation policy.
pub(crate) struct VerifiedBackup {
    path: PathBuf,
}

impl VerifiedBackup {
    /// Returns the retained path for crate-private migration-safety tests.
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Creates, verifies, and retains an online backup before one migration.
pub(crate) fn create_verified_backup(
    source: &Connection,
    database_path: &Path,
    from_version: u32,
    to_version: u32,
) -> Result<VerifiedBackup, StorageError> {
    let path = reserve_backup_path(database_path, from_version, to_version)?;
    if run_online_backup(source, &path).is_err() {
        let _ = fs::remove_file(&path);
        return Err(StorageError::BackupCreationFailed);
    }
    verify_backup(&path)?;
    Ok(VerifiedBackup { path })
}

fn run_online_backup(source: &Connection, path: &Path) -> Result<(), StorageError> {
    let mut destination = Connection::open(path).map_err(|_| StorageError::BackupCreationFailed)?;
    {
        let backup = Backup::new(source, &mut destination)
            .map_err(|_| StorageError::BackupCreationFailed)?;
        loop {
            match backup
                .step(100)
                .map_err(|_| StorageError::BackupCreationFailed)?
            {
                StepResult::Done => break,
                StepResult::More => {}
                StepResult::Busy | StepResult::Locked => {
                    return Err(StorageError::BackupCreationFailed);
                }
                _ => return Err(StorageError::BackupCreationFailed),
            }
        }
    }
    destination
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")
        .map_err(|_| StorageError::BackupCreationFailed)
}

/// Restores a retained verified backup into the existing writer connection.
///
/// The caller must use this only after migration work failed. The backup is
/// retained after the restore so recovery evidence is not silently discarded.
pub(crate) fn restore_verified_backup(
    destination: &mut Connection,
    backup: &VerifiedBackup,
) -> Result<(), StorageError> {
    destination
        .restore("main", &backup.path, None::<fn(rusqlite::backup::Progress)>)
        .map_err(|_| StorageError::BackupRestoreFailed)?;
    connection::configure_writer(destination)?;
    verify_database_integrity(destination).map_err(restored_database_integrity_error)
}

fn reserve_backup_path(
    database_path: &Path,
    from_version: u32,
    to_version: u32,
) -> Result<PathBuf, StorageError> {
    let directory = database_path.parent().unwrap_or_else(|| Path::new("."));
    let filename = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StorageError::BackupPathUnavailable)?;

    for suffix in 0_u32..=u32::MAX {
        let disambiguator = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        // The deterministic versioned name is the retained recovery path. A
        // collision gets a numeric suffix so an earlier recovery artifact is
        // never overwritten by a later migration attempt.
        let path = directory.join(format!(
            "{filename}.pre-migration-v{from_version}-to-v{to_version}{disambiguator}.sqlite3"
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                if file.sync_all().is_err() {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(StorageError::BackupPathUnavailable);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(_) => return Err(StorageError::BackupPathUnavailable),
        }
    }

    Err(StorageError::BackupPathUnavailable)
}

fn verify_backup(path: &Path) -> Result<(), StorageError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| StorageError::BackupVerificationOpenFailed)?;
    verify_database_integrity(&connection).map_err(backup_integrity_error)
}

/// Verifies a complete SQLite image after migration or unclean recovery work.
///
/// Migration execution and future unclean-recovery ownership both use this
/// crate-private primitive so the integrity policy cannot drift by call site.
pub(crate) fn verify_database_integrity(
    connection: &Connection,
) -> Result<(), IntegrityCheckFailure> {
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|_| IntegrityCheckFailure::QuickCheck)?;
    if !quick_check.eq_ignore_ascii_case("ok") {
        return Err(IntegrityCheckFailure::QuickCheck);
    }

    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| IntegrityCheckFailure::ForeignKeyCheck)?;
    let mut rows = statement
        .query([])
        .map_err(|_| IntegrityCheckFailure::ForeignKeyCheck)?;
    match rows
        .next()
        .map_err(|_| IntegrityCheckFailure::ForeignKeyCheck)?
    {
        Some(_) => Err(IntegrityCheckFailure::ForeignKeyCheck),
        None => Ok(()),
    }
}

fn backup_integrity_error(failure: IntegrityCheckFailure) -> StorageError {
    match failure {
        IntegrityCheckFailure::QuickCheck => StorageError::BackupQuickCheckFailed,
        IntegrityCheckFailure::ForeignKeyCheck => StorageError::BackupForeignKeyCheckFailed,
    }
}

fn restored_database_integrity_error(failure: IntegrityCheckFailure) -> StorageError {
    match failure {
        IntegrityCheckFailure::QuickCheck => StorageError::RestoredDatabaseQuickCheckFailed,
        IntegrityCheckFailure::ForeignKeyCheck => {
            StorageError::RestoredDatabaseForeignKeyCheckFailed
        }
    }
}
