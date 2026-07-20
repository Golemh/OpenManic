//! Data-root resolution and writer ownership before SQLite opens.
//!
//! This module owns only bootstrap metadata and filesystem probes. It does not open SQLite or
//! store substantive application data. Callers keep the returned
//! [`DataRootLock`](crate::data_root::DataRootLock) alive for the entire writer lifetime, making
//! ownership explicit rather than using a process-global locator.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

/// The directory created beside a writable release artifact by default.
pub const ARTIFACT_DATA_DIRECTORY: &str = "OpenManicData";
const LOCATOR_SCHEMA_VERSION: u32 = 1;
const LOCK_FILE_NAME: &str = ".openmanic-data-root.lock";
#[cfg(windows)]
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
static PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where the selected data root came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRootSource {
    /// The `--data-dir` command-line override.
    CommandLine,
    /// The `OPENMANIC_DATA_DIR` environment override.
    Environment,
    /// A prior user-selected per-user bootstrap locator.
    BootstrapLocator,
    /// The writable directory beside the actual artifact.
    ArtifactAdjacent,
}

impl DataRootSource {
    /// Returns the stable source name used by diagnostics.
    #[must_use]
    pub const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::CommandLine => "command-line",
            Self::Environment => "environment",
            Self::BootstrapLocator => "bootstrap-locator",
            Self::ArtifactAdjacent => "artifact-adjacent",
        }
    }
}

/// An accepted data-root path with the source that selected it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedDataRoot {
    path: PathBuf,
    source: DataRootSource,
}

impl ResolvedDataRoot {
    /// Returns the validated data-root path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the source that selected this path.
    #[must_use]
    pub const fn source(&self) -> DataRootSource {
        self.source
    }

    /// Records a successfully moved root as selected by the persisted bootstrap locator.
    #[must_use]
    pub fn moved(root: PathBuf) -> Self {
        Self {
            path: root,
            source: DataRootSource::BootstrapLocator,
        }
    }
}

/// Inputs considered in the documented resolution order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DataRootInputs {
    /// An explicit command-line override.
    pub command_line: Option<PathBuf>,
    /// The advanced environment override.
    pub environment: Option<PathBuf>,
    /// A valid, prior user-selected locator.
    pub locator: Option<BootstrapLocator>,
}

/// A reason to present a blocking directory chooser instead of creating another store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryChoiceReason {
    /// No prior source was usable and the artifact-adjacent root could not be validated.
    ArtifactDirectoryUnavailable,
    /// A previous custom data root is no longer usable and needs explicit recovery.
    LocatorDirectoryUnavailable,
}

/// A deferred root choice that must be presented before tracking starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryChoiceRequest {
    reason: DirectoryChoiceReason,
}

impl DirectoryChoiceRequest {
    /// Returns why the caller must show the directory chooser.
    #[must_use]
    pub const fn reason(self) -> DirectoryChoiceReason {
        self.reason
    }
}

/// The deterministic result of data-root resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataRootResolution {
    /// A validated root was selected and can next be locked for the process lifetime.
    Ready(ResolvedDataRoot),
    /// User choice is required before any store is created or tracking is claimed active.
    DirectoryChoiceRequired(DirectoryChoiceRequest),
}

/// A privacy-safe filesystem failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSystemFailure {
    /// Access was denied by the filesystem or platform.
    AccessDenied,
    /// A required parent or path does not exist.
    NotFound,
    /// A path already exists when a unique marker was required.
    AlreadyExists,
    /// The operation failed for another local filesystem reason.
    Other,
}

impl From<&io::Error> for FileSystemFailure {
    fn from(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::PermissionDenied => Self::AccessDenied,
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            _ => Self::Other,
        }
    }
}

/// A validation failure that is safe to surface without embedding a selected path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRootValidationError {
    /// The path was empty.
    EmptyPath,
    /// The path exists but is not a directory.
    NotDirectory,
    /// A platform-specific seam identified the root as a network share.
    NetworkShare,
    /// A create/write/rename/delete probe failed.
    FileSystem(FileSystemFailure),
    /// The SQLite-WAL-shaped filesystem probe failed.
    WalCompatibility(FileSystemFailure),
    /// An exclusive writer lock could not be acquired.
    LockUnavailable(FileSystemFailure),
}

impl DataRootValidationError {
    /// Returns a stable code for diagnostics and UI recovery mapping.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyPath => "bootstrap.data-root.empty-path",
            Self::NotDirectory => "bootstrap.data-root.not-directory",
            Self::NetworkShare => "bootstrap.data-root.network-share",
            Self::FileSystem(_) => "bootstrap.data-root.filesystem",
            Self::WalCompatibility(_) => "bootstrap.data-root.wal-incompatible",
            Self::LockUnavailable(_) => "bootstrap.data-root.lock-unavailable",
        }
    }
}

/// A resolution failure that must not silently fall through to a fresh data root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRootResolutionError {
    /// The explicit command-line override was not usable.
    InvalidCommandLineOverride(DataRootValidationError),
    /// The explicit environment override was not usable.
    InvalidEnvironmentOverride(DataRootValidationError),
}

impl DataRootResolutionError {
    /// Returns the stable code for this recovery condition.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidCommandLineOverride(_) => {
                "bootstrap.data-root.invalid-command-line-override"
            }
            Self::InvalidEnvironmentOverride(_) => {
                "bootstrap.data-root.invalid-environment-override"
            }
        }
    }
}

/// Validates a candidate root without exposing platform APIs to bootstrap policy.
pub trait DataRootValidator {
    /// Validates whether a directory can safely host the local OpenManic store.
    ///
    /// # Errors
    ///
    /// Returns a privacy-safe reason when the path cannot host a local writable, WAL-compatible,
    /// lockable store.
    fn validate(&self, root: &Path) -> Result<(), DataRootValidationError>;
}

/// Platform seam that identifies network shares before a store is opened.
pub trait NetworkShareValidator {
    /// Rejects roots that are network shares according to the active platform policy.
    ///
    /// # Errors
    ///
    /// Returns [`DataRootValidationError::NetworkShare`] when the path is not local.
    fn require_local(&self, root: &Path) -> Result<(), DataRootValidationError>;
}

/// Conservative cross-platform detector for commonly spelled network-share paths.
#[derive(Clone, Copy, Debug, Default)]
pub struct RejectKnownNetworkShares;

impl NetworkShareValidator for RejectKnownNetworkShares {
    fn require_local(&self, root: &Path) -> Result<(), DataRootValidationError> {
        let spelling = root.as_os_str().to_string_lossy();
        if spelling.starts_with(r"\\") || spelling.starts_with("//") {
            return Err(DataRootValidationError::NetworkShare);
        }
        Ok(())
    }
}

/// Local filesystem validation with an injectable network-share policy.
#[derive(Clone, Debug)]
pub struct LocalDataRootValidator<N> {
    network_share_validator: N,
}

impl<N> LocalDataRootValidator<N> {
    /// Creates validation using the supplied platform network-share policy.
    #[must_use]
    pub const fn new(network_share_validator: N) -> Self {
        Self {
            network_share_validator,
        }
    }
}

impl Default for LocalDataRootValidator<RejectKnownNetworkShares> {
    fn default() -> Self {
        Self::new(RejectKnownNetworkShares)
    }
}

impl<N> DataRootValidator for LocalDataRootValidator<N>
where
    N: NetworkShareValidator,
{
    fn validate(&self, root: &Path) -> Result<(), DataRootValidationError> {
        if root.as_os_str().is_empty() {
            return Err(DataRootValidationError::EmptyPath);
        }
        self.network_share_validator.require_local(root)?;
        fs::create_dir_all(root)
            .map_err(|error| DataRootValidationError::FileSystem((&error).into()))?;
        if !fs::metadata(root)
            .map_err(|error| DataRootValidationError::FileSystem((&error).into()))?
            .is_dir()
        {
            return Err(DataRootValidationError::NotDirectory);
        }

        probe_write_rename_delete(root, "probe")
            .map_err(|error| DataRootValidationError::FileSystem((&error).into()))?;
        probe_write_rename_delete(root, "wal-probe")
            .map_err(|error| DataRootValidationError::WalCompatibility((&error).into()))?;
        let lock = DataRootLock::acquire(root)
            .map_err(|error| DataRootValidationError::LockUnavailable(error.failure()))?;
        drop(lock);
        Ok(())
    }
}

/// Resolves the data root with the documented precedence.
///
/// Invalid explicit overrides return an error and do not fall back to a new root. An unavailable
/// locator or artifact-adjacent directory yields a chooser request so the user makes recovery
/// explicit before any SQLite store is created.
///
/// # Errors
///
/// Returns [`DataRootResolutionError`] when an explicit command-line or environment override is
/// invalid. Callers should present recovery, not silently create storage elsewhere.
pub fn resolve_data_root<V>(
    inputs: &DataRootInputs,
    artifact_directory: &Path,
    validator: &V,
) -> Result<DataRootResolution, DataRootResolutionError>
where
    V: DataRootValidator,
{
    if let Some(path) = &inputs.command_line {
        return validate_explicit(path, DataRootSource::CommandLine, validator);
    }
    if let Some(path) = &inputs.environment {
        return validate_explicit(path, DataRootSource::Environment, validator);
    }
    if let Some(locator) = &inputs.locator {
        if !locator.data_root().is_dir() {
            return Ok(DataRootResolution::DirectoryChoiceRequired(
                DirectoryChoiceRequest {
                    reason: DirectoryChoiceReason::LocatorDirectoryUnavailable,
                },
            ));
        }
        return match validator.validate(locator.data_root()) {
            Ok(()) => Ok(DataRootResolution::Ready(ResolvedDataRoot {
                path: locator.data_root().to_path_buf(),
                source: DataRootSource::BootstrapLocator,
            })),
            Err(_) => Ok(DataRootResolution::DirectoryChoiceRequired(
                DirectoryChoiceRequest {
                    reason: DirectoryChoiceReason::LocatorDirectoryUnavailable,
                },
            )),
        };
    }

    let artifact_root = artifact_directory.join(ARTIFACT_DATA_DIRECTORY);
    match validator.validate(&artifact_root) {
        Ok(()) => Ok(DataRootResolution::Ready(ResolvedDataRoot {
            path: artifact_root,
            source: DataRootSource::ArtifactAdjacent,
        })),
        Err(_) => Ok(DataRootResolution::DirectoryChoiceRequired(
            DirectoryChoiceRequest {
                reason: DirectoryChoiceReason::ArtifactDirectoryUnavailable,
            },
        )),
    }
}

fn validate_explicit<V>(
    path: &Path,
    source: DataRootSource,
    validator: &V,
) -> Result<DataRootResolution, DataRootResolutionError>
where
    V: DataRootValidator,
{
    validator.validate(path).map_err(|error| match source {
        DataRootSource::CommandLine => DataRootResolutionError::InvalidCommandLineOverride(error),
        DataRootSource::Environment => DataRootResolutionError::InvalidEnvironmentOverride(error),
        DataRootSource::BootstrapLocator | DataRootSource::ArtifactAdjacent => {
            unreachable!("only explicit data-root sources are validated by this helper")
        }
    })?;
    Ok(DataRootResolution::Ready(ResolvedDataRoot {
        path: path.to_path_buf(),
        source,
    }))
}

/// Process-lifetime ownership of the selected data-root writer slot.
///
/// Dropping this value removes its lock marker. The guard must be retained until the writer has
/// stopped, as it is the fallback writer-protection path when instance activation fails.
#[derive(Debug)]
pub struct DataRootLock {
    path: PathBuf,
    _file: File,
}

impl DataRootLock {
    /// Attempts to acquire the exclusive local data-root writer lock.
    ///
    /// # Errors
    ///
    /// Returns [`DataRootLockError`] when another process owns the root or the marker cannot be
    /// created. The error intentionally omits the root path.
    pub fn acquire(root: &Path) -> Result<Self, DataRootLockError> {
        let path = root.join(LOCK_FILE_NAME);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        // Windows closes all process handles during normal shutdown and abnormal termination.
        // Tying removal to that handle lifecycle prevents a dead process from stranding a
        // marker that would otherwise block the next launch.
        #[cfg(windows)]
        options.custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
        let file = options.open(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                DataRootLockError::AlreadyLocked
            } else {
                DataRootLockError::FileSystem((&error).into())
            }
        })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for DataRootLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Failure to acquire the writer lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRootLockError {
    /// Another process already owns this data root.
    AlreadyLocked,
    /// The local lock marker could not be created.
    FileSystem(FileSystemFailure),
}

/// Coordinates the store-specific portions of a data-root move.
///
/// The caller owns the live SQLite writer and its [`DataRootLock`].  It must checkpoint and close
/// that writer before returning from [`DataRootMoveSession::quiesce_and_close`], then use its
/// accepted backup/verification machinery to prove the copied store can be reopened.  Keeping
/// those operations behind this small seam prevents bootstrap code from opening a second writer.
pub trait DataRootMoveSession {
    /// Stops capture, checkpoints the store, releases its writer lock, and closes all database
    /// handles associated with the source root.
    ///
    /// # Errors
    ///
    /// Returns [`DataRootMoveSessionError`] when the source cannot be safely quiesced and closed.
    fn quiesce_and_close(&mut self) -> Result<(), DataRootMoveSessionError>;

    /// Verifies the copied store at `destination` without trusting the filesystem copy alone.
    ///
    /// # Errors
    ///
    /// Returns [`DataRootMoveSessionError`] when storage verification rejects the copied store.
    fn verify_destination(&mut self, destination: &Path) -> Result<(), DataRootMoveSessionError>;

    /// Reopens the accepted store at the verified destination.
    ///
    /// # Errors
    ///
    /// Returns [`DataRootMoveSessionError`] when the verified destination cannot be reopened.
    fn reopen_destination(&mut self, destination: &Path) -> Result<(), DataRootMoveSessionError>;

    /// Reopens the retained source after a failed move before locator switch.
    ///
    /// # Errors
    ///
    /// Returns [`DataRootMoveSessionError`] when recovery cannot reopen the retained source.
    fn reopen_source(&mut self, source: &Path) -> Result<(), DataRootMoveSessionError>;
}

/// An opaque storage-session failure during a coordinated data-root move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRootMoveSessionError {
    /// The session could not complete its requested lifecycle operation.
    Failed,
}

/// Failure category for a coordinated data-directory move.
///
/// Paths and database contents intentionally remain out of this error because it is suitable for
/// user-visible recovery and diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataRootMoveError {
    /// Source and destination identify the same requested directory.
    SameDirectory,
    /// The source is not an existing directory.
    SourceUnavailable,
    /// The destination contains data and therefore cannot be safely overwritten.
    DestinationNotEmpty,
    /// The destination failed the normal local/WAL/lock validation.
    DestinationValidation(DataRootValidationError),
    /// The active writer could not be safely quiesced before copying.
    QuiesceFailed,
    /// Copying a source entry to the destination failed.
    CopyFailed(FileSystemFailure),
    /// The copied store did not pass the storage-owned verification procedure.
    VerificationFailed,
    /// The verified store could not be reopened at the destination.
    ReopenDestinationFailed,
    /// The bootstrap locator could not be atomically switched after a successful reopen.
    LocatorSwitchFailed(LocatorError),
    /// Recovery to the retained source failed after the locator was left unchanged.
    SourceRecoveryFailed,
}

impl DataRootMoveError {
    /// Returns the stable, privacy-safe diagnostic code for this recovery condition.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::SameDirectory => "data-move.same-directory",
            Self::SourceUnavailable => "data-move.source-unavailable",
            Self::DestinationNotEmpty => "data-move.destination-not-empty",
            Self::DestinationValidation(_) => "data-move.destination-invalid",
            Self::QuiesceFailed => "data-move.quiesce-failed",
            Self::CopyFailed(_) => "data-move.copy-failed",
            Self::VerificationFailed => "data-move.verification-failed",
            Self::ReopenDestinationFailed => "data-move.destination-reopen-failed",
            Self::LocatorSwitchFailed(_) => "data-move.locator-switch-failed",
            Self::SourceRecoveryFailed => "data-move.source-recovery-failed",
        }
    }
}

/// Moves a closed data root, verifies it through the active storage session, then atomically
/// switches the bootstrap locator.
///
/// The destination must be empty.  The source is never removed by this operation: it remains
/// available for recovery after every failure and after successful locator switch.  The caller
/// must obtain user confirmation before calling this destructive-location operation.
///
/// # Errors
///
/// Returns a privacy-safe failure category.  The locator remains unchanged unless this function
/// returns `Ok`; if locator persistence fails, it attempts to reopen the retained source.
pub fn move_data_root<V, S>(
    source: &Path,
    destination: &Path,
    locator_path: &Path,
    store_id: Option<StoreId>,
    validator: &V,
    session: &mut S,
) -> Result<BootstrapLocator, DataRootMoveError>
where
    V: DataRootValidator,
    S: DataRootMoveSession,
{
    if source == destination {
        return Err(DataRootMoveError::SameDirectory);
    }
    if !source.is_dir() {
        return Err(DataRootMoveError::SourceUnavailable);
    }
    validator
        .validate(destination)
        .map_err(DataRootMoveError::DestinationValidation)?;
    if fs::read_dir(destination)
        .map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?
        .next()
        .is_some()
    {
        return Err(DataRootMoveError::DestinationNotEmpty);
    }

    session
        .quiesce_and_close()
        .map_err(|_| DataRootMoveError::QuiesceFailed)?;
    copy_data_root_contents(source, destination)?;
    if session.verify_destination(destination).is_err() {
        return recover_source(session, source, DataRootMoveError::VerificationFailed);
    }
    if session.reopen_destination(destination).is_err() {
        return recover_source(session, source, DataRootMoveError::ReopenDestinationFailed);
    }

    let locator = BootstrapLocator::new(destination.to_path_buf(), store_id);
    if let Err(error) = persist_locator(locator_path, &locator) {
        return recover_source(
            session,
            source,
            DataRootMoveError::LocatorSwitchFailed(error),
        );
    }
    Ok(locator)
}

fn recover_source<S>(
    session: &mut S,
    source: &Path,
    original_error: DataRootMoveError,
) -> Result<BootstrapLocator, DataRootMoveError>
where
    S: DataRootMoveSession,
{
    session
        .reopen_source(source)
        .map_err(|_| DataRootMoveError::SourceRecoveryFailed)?;
    Err(original_error)
}

fn copy_data_root_contents(source: &Path, destination: &Path) -> Result<(), DataRootMoveError> {
    for entry in
        fs::read_dir(source).map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?
    {
        let entry = entry.map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?;
        let source_path = entry.path();
        if source_path
            .file_name()
            .is_some_and(|name| name == LOCK_FILE_NAME)
        {
            continue;
        }
        let destination_path = destination.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?;
        if metadata.is_dir() {
            fs::create_dir(&destination_path)
                .map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?;
            copy_data_root_contents(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)
                .map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&destination_path)
                .and_then(|file| file.sync_all())
                .map_err(|error| DataRootMoveError::CopyFailed((&error).into()))?;
        }
    }
    Ok(())
}

impl DataRootLockError {
    fn failure(self) -> FileSystemFailure {
        match self {
            Self::AlreadyLocked => FileSystemFailure::AlreadyExists,
            Self::FileSystem(failure) => failure,
        }
    }
}

/// A stable identifier for a selected store, kept only in the bootstrap locator when known.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreId(String);

impl StoreId {
    /// Validates an opaque stable store identifier.
    ///
    /// # Errors
    ///
    /// Returns [`LocatorError::Malformed`] for an empty or line-breaking identifier.
    pub fn new(value: String) -> Result<Self, LocatorError> {
        if value.is_empty() || value.contains(['\n', '\r', '\0']) {
            return Err(LocatorError::Malformed);
        }
        Ok(Self(value))
    }

    /// Returns the opaque stable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The deliberately tiny per-user custom-data-root locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapLocator {
    data_root: PathBuf,
    store_id: Option<StoreId>,
}

impl BootstrapLocator {
    /// Creates a locator for a user-selected root and optional known store identifier.
    #[must_use]
    pub fn new(data_root: PathBuf, store_id: Option<StoreId>) -> Self {
        Self {
            data_root,
            store_id,
        }
    }

    /// Returns the selected root without opening the store.
    #[must_use]
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Returns the known store identifier, when the store has been initialized.
    #[must_use]
    pub fn store_id(&self) -> Option<&StoreId> {
        self.store_id.as_ref()
    }
}

/// A locator persistence or parse failure without user paths or data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocatorError {
    /// The locator could not be read or persisted locally.
    FileSystem(FileSystemFailure),
    /// The locator did not have the exact small bootstrap schema.
    Malformed,
    /// The locator schema version is not supported by this binary.
    UnsupportedSchema,
    /// The selected path cannot be represented as UTF-8 in this portable locator format.
    NonUnicodePath,
}

impl LocatorError {
    /// Returns the stable diagnostic code for locator recovery.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FileSystem(_) => "bootstrap.locator.filesystem",
            Self::Malformed => "bootstrap.locator.malformed",
            Self::UnsupportedSchema => "bootstrap.locator.unsupported-schema",
            Self::NonUnicodePath => "bootstrap.locator.non-unicode-path",
        }
    }
}

/// Loads the small per-user locator if it exists.
///
/// # Errors
///
/// Returns [`LocatorError`] for malformed or unreadable existing locators. A missing file is a
/// normal first-launch condition and returns `Ok(None)`.
pub fn load_locator(locator_path: &Path) -> Result<Option<BootstrapLocator>, LocatorError> {
    let content = match fs::read_to_string(locator_path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(LocatorError::FileSystem((&error).into())),
    };
    parse_locator(&content).map(Some)
}

/// Atomically persists only the selected path, schema version, and optional store identifier.
///
/// # Errors
///
/// Returns [`LocatorError`] if the locator cannot be safely written and renamed within its own
/// directory. It never writes substantive application data outside the selected root.
pub fn persist_locator(
    locator_path: &Path,
    locator: &BootstrapLocator,
) -> Result<(), LocatorError> {
    let parent = locator_path.parent().ok_or(LocatorError::Malformed)?;
    fs::create_dir_all(parent).map_err(|error| LocatorError::FileSystem((&error).into()))?;
    let encoded = serialize_locator(locator)?;
    let temporary_path = parent.join(unique_name("locator"));
    let mut temporary = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| LocatorError::FileSystem((&error).into()))?;
    let result = (|| -> Result<(), LocatorError> {
        temporary
            .write_all(encoded.as_bytes())
            .map_err(|error| LocatorError::FileSystem((&error).into()))?;
        temporary
            .sync_all()
            .map_err(|error| LocatorError::FileSystem((&error).into()))?;
        drop(temporary);
        fs::rename(&temporary_path, locator_path)
            .map_err(|error| LocatorError::FileSystem((&error).into()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn parse_locator(content: &str) -> Result<BootstrapLocator, LocatorError> {
    let mut schema_version = None;
    let mut encoded_root = None;
    let mut encoded_store_id = None;

    for line in content.lines() {
        let (key, value) = line.split_once('=').ok_or(LocatorError::Malformed)?;
        match key {
            "schema_version" if schema_version.is_none() => schema_version = Some(value),
            "data_root" if encoded_root.is_none() => encoded_root = Some(value),
            "store_id" if encoded_store_id.is_none() => encoded_store_id = Some(value),
            _ => return Err(LocatorError::Malformed),
        }
    }

    let schema_version = schema_version.ok_or(LocatorError::Malformed)?;
    if schema_version != LOCATOR_SCHEMA_VERSION.to_string() {
        return Err(LocatorError::UnsupportedSchema);
    }
    let data_root = decode_value(encoded_root.ok_or(LocatorError::Malformed)?)?;
    if data_root.is_empty() {
        return Err(LocatorError::Malformed);
    }
    let store_id = match encoded_store_id {
        Some(value) => Some(StoreId::new(decode_value(value)?)?),
        None => None,
    };
    Ok(BootstrapLocator::new(PathBuf::from(data_root), store_id))
}

fn serialize_locator(locator: &BootstrapLocator) -> Result<String, LocatorError> {
    let root = locator
        .data_root()
        .to_str()
        .ok_or(LocatorError::NonUnicodePath)?;
    let mut serialized = format!(
        "schema_version={LOCATOR_SCHEMA_VERSION}\ndata_root={}\n",
        encode_value(root)
    );
    if let Some(store_id) = locator.store_id() {
        serialized.push_str("store_id=");
        serialized.push_str(&encode_value(store_id.as_str()));
        serialized.push('\n');
    }
    Ok(serialized)
}

fn encode_value(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_value(encoded: &str) -> Result<String, LocatorError> {
    if !encoded.len().is_multiple_of(2) {
        return Err(LocatorError::Malformed);
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for chunk in encoded.as_bytes().chunks_exact(2) {
        let chunk = std::str::from_utf8(chunk).map_err(|_| LocatorError::Malformed)?;
        bytes.push(u8::from_str_radix(chunk, 16).map_err(|_| LocatorError::Malformed)?);
    }
    String::from_utf8(bytes).map_err(|_| LocatorError::Malformed)
}

fn probe_write_rename_delete(root: &Path, prefix: &str) -> io::Result<()> {
    let created = root.join(unique_name(prefix));
    let renamed = root.join(unique_name(prefix));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&created)?;
    file.write_all(b"openmanic-bootstrap-probe")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&created, &renamed)?;
    fs::remove_file(&renamed)
}

fn unique_name(prefix: &str) -> String {
    let number = PROBE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(".openmanic-{prefix}-{}-{number}", std::process::id())
}

impl fmt::Display for DataRootResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DataRootResolutionError {}

#[cfg(test)]
mod tests {
    use super::{
        ARTIFACT_DATA_DIRECTORY, BootstrapLocator, DataRootInputs, DataRootLock, DataRootMoveError,
        DataRootMoveSession, DataRootMoveSessionError, DataRootResolution, DataRootResolutionError,
        DataRootSource, DataRootValidationError, DataRootValidator, DirectoryChoiceReason,
        FileSystemFailure, LocalDataRootValidator, LocatorError, NetworkShareValidator,
        RejectKnownNetworkShares, StoreId, load_locator, move_data_root, persist_locator,
        resolve_data_root,
    };
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct FakeValidator {
        rejected: BTreeSet<PathBuf>,
    }

    impl FakeValidator {
        fn rejecting(paths: &[PathBuf]) -> Self {
            Self {
                rejected: paths.iter().cloned().collect(),
            }
        }
    }

    impl DataRootValidator for FakeValidator {
        fn validate(&self, root: &Path) -> Result<(), DataRootValidationError> {
            if self.rejected.contains(root) {
                return Err(DataRootValidationError::NetworkShare);
            }
            Ok(())
        }
    }

    struct CountingValidator {
        calls: Cell<usize>,
    }

    impl CountingValidator {
        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    impl DataRootValidator for CountingValidator {
        fn validate(&self, _: &Path) -> Result<(), DataRootValidationError> {
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MoveSession {
        calls: Vec<&'static str>,
        fail_verification: bool,
    }

    impl DataRootMoveSession for MoveSession {
        fn quiesce_and_close(&mut self) -> Result<(), DataRootMoveSessionError> {
            self.calls.push("quiesce");
            Ok(())
        }

        fn verify_destination(&mut self, _: &Path) -> Result<(), DataRootMoveSessionError> {
            self.calls.push("verify");
            if self.fail_verification {
                Err(DataRootMoveSessionError::Failed)
            } else {
                Ok(())
            }
        }

        fn reopen_destination(&mut self, _: &Path) -> Result<(), DataRootMoveSessionError> {
            self.calls.push("reopen-destination");
            Ok(())
        }

        fn reopen_source(&mut self, _: &Path) -> Result<(), DataRootMoveSessionError> {
            self.calls.push("reopen-source");
            Ok(())
        }
    }

    fn unique_test_directory(name: &str) -> PathBuf {
        let number = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("openmanic-{name}-{}-{number}", std::process::id()))
    }

    #[test]
    fn command_line_override_precedes_every_other_source() {
        let inputs = DataRootInputs {
            command_line: Some(PathBuf::from("command")),
            environment: Some(PathBuf::from("environment")),
            locator: Some(BootstrapLocator::new(PathBuf::from("locator"), None)),
        };

        let resolution =
            resolve_data_root(&inputs, Path::new("artifact"), &FakeValidator::default())
                .expect("command line root resolves");

        assert_eq!(
            resolution,
            DataRootResolution::Ready(super::ResolvedDataRoot {
                path: PathBuf::from("command"),
                source: DataRootSource::CommandLine,
            })
        );
    }

    #[test]
    fn environment_precedes_locator_when_command_line_is_absent() {
        let inputs = DataRootInputs {
            command_line: None,
            environment: Some(PathBuf::from("environment")),
            locator: Some(BootstrapLocator::new(PathBuf::from("locator"), None)),
        };

        let resolution =
            resolve_data_root(&inputs, Path::new("artifact"), &FakeValidator::default())
                .expect("environment root resolves");

        assert!(matches!(
            resolution,
            DataRootResolution::Ready(ref root)
                if root.path() == Path::new("environment")
                    && root.source() == DataRootSource::Environment
        ));
    }

    #[test]
    fn invalid_command_line_override_is_not_replaced_by_a_new_root() {
        let command_line = PathBuf::from("invalid-command");
        let inputs = DataRootInputs {
            command_line: Some(command_line.clone()),
            environment: Some(PathBuf::from("environment")),
            locator: None,
        };
        let validator = FakeValidator::rejecting(&[command_line]);

        let error = resolve_data_root(&inputs, Path::new("artifact"), &validator)
            .expect_err("invalid explicit override must stop resolution");

        assert_eq!(
            error,
            DataRootResolutionError::InvalidCommandLineOverride(
                DataRootValidationError::NetworkShare
            )
        );
    }

    #[test]
    fn unavailable_locator_requires_explicit_directory_choice() {
        let locator_path = PathBuf::from("missing-custom-root");
        let inputs = DataRootInputs {
            command_line: None,
            environment: None,
            locator: Some(BootstrapLocator::new(locator_path.clone(), None)),
        };
        let validator = CountingValidator {
            calls: Cell::new(0),
        };

        let resolution = resolve_data_root(&inputs, Path::new("artifact"), &validator)
            .expect("a locator failure asks for recovery");

        assert_eq!(
            resolution,
            DataRootResolution::DirectoryChoiceRequired(super::DirectoryChoiceRequest {
                reason: DirectoryChoiceReason::LocatorDirectoryUnavailable,
            })
        );
        assert_eq!(validator.calls(), 0);
    }

    #[test]
    fn locator_precedes_artifact_default_when_its_directory_is_valid() {
        let locator_directory = unique_test_directory("locator-precedence");
        fs::create_dir_all(&locator_directory).expect("locator directory is created");
        let inputs = DataRootInputs {
            command_line: None,
            environment: None,
            locator: Some(BootstrapLocator::new(locator_directory.clone(), None)),
        };

        let resolution =
            resolve_data_root(&inputs, Path::new("artifact"), &FakeValidator::default())
                .expect("valid locator resolves");

        assert!(matches!(
            resolution,
            DataRootResolution::Ready(ref root)
                if root.path() == locator_directory && root.source() == DataRootSource::BootstrapLocator
        ));
        fs::remove_dir_all(locator_directory).expect("locator test root is removed");
    }

    #[test]
    fn unwritable_artifact_default_requires_directory_choice() {
        let artifact = PathBuf::from("artifact");
        let unavailable_default = artifact.join(ARTIFACT_DATA_DIRECTORY);
        let validator = FakeValidator::rejecting(&[unavailable_default]);

        let resolution = resolve_data_root(&DataRootInputs::default(), &artifact, &validator)
            .expect("default failure asks for directory choice");

        assert!(matches!(
            resolution,
            DataRootResolution::DirectoryChoiceRequired(choice)
                if choice.reason() == DirectoryChoiceReason::ArtifactDirectoryUnavailable
        ));
    }

    #[test]
    fn lock_prevents_second_writer_until_first_guard_drops() {
        let directory = unique_test_directory("lock");
        fs::create_dir_all(&directory).expect("test root is created");

        let first = DataRootLock::acquire(&directory).expect("first writer acquires lock");
        let second = DataRootLock::acquire(&directory).expect_err("second writer is rejected");
        assert_eq!(second, super::DataRootLockError::AlreadyLocked);
        drop(first);
        let third = DataRootLock::acquire(&directory).expect("released lock can be acquired");
        drop(third);
        fs::remove_dir_all(directory).expect("test root is removed");
    }

    #[test]
    fn local_validation_runs_writable_wal_and_lock_probes_without_leaving_markers() {
        let directory = unique_test_directory("local-validation");
        let validator = LocalDataRootValidator::default();

        validator
            .validate(&directory)
            .expect("local temporary directory is writable and lockable");
        let remaining_entries = fs::read_dir(&directory)
            .expect("validated directory is readable")
            .count();

        assert_eq!(remaining_entries, 0);
        fs::remove_dir_all(directory).expect("validated test root is removed");
    }

    #[test]
    fn known_network_share_spelling_is_rejected_before_store_creation() {
        let error = RejectKnownNetworkShares
            .require_local(Path::new("//server/openmanic"))
            .expect_err("network share must be rejected");

        assert_eq!(error, DataRootValidationError::NetworkShare);
    }

    #[test]
    fn locator_round_trip_persists_only_bootstrap_metadata() {
        let directory = unique_test_directory("locator");
        let locator_path = directory.join("bootstrap.locator");
        let root = PathBuf::from("C:/OpenManic Data");
        let store_id = StoreId::new("store-123".to_owned()).expect("stable ID is valid");
        let locator = BootstrapLocator::new(root.clone(), Some(store_id));

        persist_locator(&locator_path, &locator).expect("locator is persisted safely");
        let replacement = BootstrapLocator::new(PathBuf::from("C:/Replacement"), None);
        persist_locator(&locator_path, &replacement)
            .expect("existing locator is atomically replaced");
        let loaded = load_locator(&locator_path)
            .expect("locator loads")
            .expect("written locator exists");
        let raw = fs::read_to_string(&locator_path).expect("locator text is readable");

        assert_eq!(loaded, replacement);
        assert!(raw.starts_with("schema_version=1\ndata_root="));
        assert!(!raw.contains("activity"));
        fs::remove_dir_all(directory).expect("test locator directory is removed");
    }

    #[test]
    fn malformed_locator_is_rejected_without_path_details() {
        let directory = unique_test_directory("malformed-locator");
        fs::create_dir_all(&directory).expect("test directory is created");
        let locator_path = directory.join("bootstrap.locator");
        fs::write(&locator_path, "unexpected=value\n").expect("malformed fixture is written");

        let error = load_locator(&locator_path).expect_err("unknown locator field is rejected");

        assert_eq!(error, LocatorError::Malformed);
        assert_eq!(
            DataRootValidationError::FileSystem(FileSystemFailure::Other).code(),
            "bootstrap.data-root.filesystem"
        );
        fs::remove_dir_all(directory).expect("test directory is removed");
    }

    #[test]
    fn data_root_move_verifies_then_switches_locator_without_removing_source() {
        let directory = unique_test_directory("data-root-move");
        let source = directory.join("source");
        let destination = directory.join("destination");
        let locator_path = directory.join("bootstrap.locator");
        fs::create_dir_all(source.join("logs")).expect("source root is created");
        fs::write(source.join("openmanic.sqlite3"), b"closed SQLite image")
            .expect("database fixture is written");
        fs::write(source.join("logs/bootstrap.log"), b"diagnostic fixture")
            .expect("nested fixture is written");
        let mut session = MoveSession::default();

        let locator = move_data_root(
            &source,
            &destination,
            &locator_path,
            Some(StoreId::new("store-123".to_owned()).expect("valid store ID")),
            &LocalDataRootValidator::default(),
            &mut session,
        )
        .expect("closed source copies, verifies, and switches locator");

        assert_eq!(locator.data_root(), destination);
        assert_eq!(
            fs::read(destination.join("openmanic.sqlite3")).expect("copied database is readable"),
            b"closed SQLite image"
        );
        assert_eq!(
            fs::read(source.join("logs/bootstrap.log")).expect("retained source is readable"),
            b"diagnostic fixture"
        );
        assert_eq!(
            load_locator(&locator_path).expect("locator loads"),
            Some(locator)
        );
        assert_eq!(session.calls, ["quiesce", "verify", "reopen-destination"]);
        fs::remove_dir_all(directory).expect("test data is removed");
    }

    #[test]
    fn data_root_move_keeps_locator_on_verification_failure_and_recovers_source() {
        let directory = unique_test_directory("data-root-move-verification");
        let source = directory.join("source");
        let destination = directory.join("destination");
        let locator_path = directory.join("bootstrap.locator");
        fs::create_dir_all(&source).expect("source root is created");
        fs::write(source.join("openmanic.sqlite3"), b"closed SQLite image")
            .expect("database fixture is written");
        let old_locator = BootstrapLocator::new(source.clone(), None);
        persist_locator(&locator_path, &old_locator).expect("old locator is persisted");
        let mut session = MoveSession {
            fail_verification: true,
            ..MoveSession::default()
        };

        let error = move_data_root(
            &source,
            &destination,
            &locator_path,
            None,
            &LocalDataRootValidator::default(),
            &mut session,
        )
        .expect_err("failed verification must keep the previous locator");

        assert_eq!(error, DataRootMoveError::VerificationFailed);
        assert_eq!(
            load_locator(&locator_path).expect("old locator remains readable"),
            Some(old_locator)
        );
        assert!(source.join("openmanic.sqlite3").is_file());
        assert_eq!(session.calls, ["quiesce", "verify", "reopen-source"]);
        fs::remove_dir_all(directory).expect("test data is removed");
    }
}
