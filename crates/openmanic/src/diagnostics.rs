//! Minimal local diagnostics that preserve privacy during process bootstrap.
//!
//! The bootstrap boundary records only fixed codes and redacted fields. Detailed diagnostics are a
//! later opt-in capability; ordinary startup output never contains window titles or selected paths.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic;
use std::path::{Path, PathBuf};

const BUNDLE_MANIFEST_FILE: &str = "diagnostics-manifest.txt";
const BUNDLE_BOOTSTRAP_LOG_FILE: &str = "bootstrap.log";
const BUNDLE_PANIC_MARKER_FILE: &str = "panic.marker";
const BUNDLE_FORMAT_VERSION: u8 = 1;

/// A fixed startup event that cannot carry arbitrary user data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticEvent {
    /// Bootstrap selected and validated a data-root source.
    DataRootResolved,
    /// Bootstrap needs a user directory choice before tracking may begin.
    DirectoryChoiceRequired,
    /// An explicit data-root override was rejected.
    DataRootOverrideRejected,
    /// A prior bootstrap locator needs user recovery.
    LocatorRecoveryRequired,
}

impl DiagnosticEvent {
    /// Returns the stable event code written to ordinary local diagnostics.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DataRootResolved => "bootstrap.data-root.resolved",
            Self::DirectoryChoiceRequired => "bootstrap.data-root.directory-choice-required",
            Self::DataRootOverrideRejected => "bootstrap.data-root.override-rejected",
            Self::LocatorRecoveryRequired => "bootstrap.locator.recovery-required",
        }
    }
}

/// Failure to initialize or append minimal local diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticError {
    /// The local diagnostics directory or file could not be used.
    FileSystem,
}

impl DiagnosticError {
    /// Returns the stable diagnostic code for this failure.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FileSystem => "bootstrap.diagnostics.filesystem",
        }
    }
}

/// Privacy-safe result of a local diagnostics-bundle export.
///
/// The bundle deliberately contains no selected data-root path, window title, application identity,
/// database content, or arbitrary log text. Its files are a fixed manifest plus validated bootstrap
/// event codes and/or a validated minimal panic marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsBundle {
    bundle_directory: PathBuf,
    included_files: Vec<&'static str>,
}

impl DiagnosticsBundle {
    /// Returns the newly created local bundle directory selected by the caller.
    #[must_use]
    pub fn bundle_directory(&self) -> &Path {
        &self.bundle_directory
    }

    /// Returns bundle-relative file names in deterministic order.
    #[must_use]
    pub fn included_files(&self) -> &[&'static str] {
        &self.included_files
    }
}

/// Failure to create a privacy-safe local diagnostics bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticsBundleError {
    /// The selected output location could not be created or written.
    FileSystem,
}

impl DiagnosticsBundleError {
    /// Returns a stable code that is safe to display in ordinary diagnostics.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FileSystem => "diagnostics.bundle.filesystem",
        }
    }
}

/// Creates a privacy-safe diagnostics bundle at a caller-selected empty destination.
///
/// The destination itself is never recorded in the bundle. Only recognized fixed bootstrap event
/// codes and a structurally valid redacted panic marker are copied. Unknown or malformed local log
/// content is intentionally omitted rather than being included as support data.
///
/// # Errors
///
/// Returns [`DiagnosticsBundleError`] if the destination cannot be created or the selected safe
/// content cannot be written.
pub fn export_diagnostics_bundle(
    data_root: &Path,
    destination: &Path,
) -> Result<DiagnosticsBundle, DiagnosticsBundleError> {
    fs::create_dir(destination).map_err(map_bundle_file_system_error)?;

    let mut included_files = vec![BUNDLE_MANIFEST_FILE];
    let bootstrap_events = read_safe_bootstrap_events(&data_root.join("logs/bootstrap.log"));
    if !bootstrap_events.is_empty() {
        fs::write(
            destination.join(BUNDLE_BOOTSTRAP_LOG_FILE),
            bootstrap_events.join("\n") + "\n",
        )
        .map_err(map_bundle_file_system_error)?;
        included_files.push(BUNDLE_BOOTSTRAP_LOG_FILE);
    }

    if let Some(marker) = read_safe_panic_marker(&data_root.join("crash/panic.marker")) {
        fs::write(destination.join(BUNDLE_PANIC_MARKER_FILE), marker)
            .map_err(map_bundle_file_system_error)?;
        included_files.push(BUNDLE_PANIC_MARKER_FILE);
    }

    let manifest = format!(
        "format_version={BUNDLE_FORMAT_VERSION}\nprivacy=validated-fixed-diagnostics-only\nfiles={}\n",
        included_files.join(",")
    );
    fs::write(destination.join(BUNDLE_MANIFEST_FILE), manifest).map_err(map_bundle_file_system_error)?;

    Ok(DiagnosticsBundle {
        bundle_directory: destination.to_owned(),
        included_files,
    })
}

/// A bounded bootstrap diagnostic writer rooted in the chosen local data directory.
#[derive(Clone, Debug)]
pub struct MinimalDiagnostics {
    log_path: PathBuf,
    crash_directory: PathBuf,
}

impl MinimalDiagnostics {
    /// Creates minimal local diagnostics under the selected data root.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError`] when the required local diagnostic directories cannot be
    /// initialized. No selected path is included in the error.
    pub fn new(data_root: &Path) -> Result<Self, DiagnosticError> {
        let log_directory = data_root.join("logs");
        let crash_directory = data_root.join("crash");
        fs::create_dir_all(&log_directory).map_err(map_file_system_error)?;
        fs::create_dir_all(&crash_directory).map_err(map_file_system_error)?;
        Ok(Self {
            log_path: log_directory.join("bootstrap.log"),
            crash_directory,
        })
    }

    /// Appends one fixed-code bootstrap event without titles, paths, or arbitrary text.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError`] if the local diagnostics file cannot be appended.
    pub fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(map_file_system_error)?;
        file.write_all(event.code().as_bytes())
            .map_err(map_file_system_error)?;
        file.write_all(b"\n").map_err(map_file_system_error)
    }

    /// Installs the minimal process panic hook before workers are started.
    ///
    /// The hook writes a fixed crash marker and source line/column only. It deliberately does not
    /// inspect ordinary service state, emit panic payload text, or include user paths or titles.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError`] when the local crash directory cannot be initialized before
    /// installing the hook.
    pub fn install_panic_hook(&self) -> Result<(), DiagnosticError> {
        fs::create_dir_all(&self.crash_directory).map_err(map_file_system_error)?;
        let crash_directory = self.crash_directory.clone();
        panic::set_hook(Box::new(move |panic_info| {
            let line = panic_info.location().map_or(0, std::panic::Location::line);
            let column = panic_info
                .location()
                .map_or(0, std::panic::Location::column);
            let _ = write_panic_marker(&crash_directory, line, column);
        }));
        Ok(())
    }

    /// Returns the local crash-marker directory for controlled bootstrap integration.
    #[must_use]
    pub fn crash_directory(&self) -> &Path {
        &self.crash_directory
    }
}

/// Replaces a path with a fixed ordinary-diagnostics token.
#[must_use]
pub const fn redact_path_for_diagnostics(_: &Path) -> &'static str {
    "[redacted-path]"
}

/// Replaces a window title with a fixed ordinary-diagnostics token.
#[must_use]
pub const fn redact_title_for_diagnostics(_: &str) -> &'static str {
    "[redacted-title]"
}

fn write_panic_marker(
    crash_directory: &Path,
    line: u32,
    column: u32,
) -> Result<(), DiagnosticError> {
    let marker_path = crash_directory.join("panic.marker");
    let marker =
        format!("event=panic\nthread=redacted\nlocation_line={line}\nlocation_column={column}\n");
    fs::write(marker_path, marker).map_err(map_file_system_error)
}

fn map_file_system_error(_: io::Error) -> DiagnosticError {
    DiagnosticError::FileSystem
}

fn read_safe_bootstrap_events(path: &Path) -> Vec<&'static str> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    content
        .lines()
        .filter_map(|line| {
            [
                DiagnosticEvent::DataRootResolved,
                DiagnosticEvent::DirectoryChoiceRequired,
                DiagnosticEvent::DataRootOverrideRejected,
                DiagnosticEvent::LocatorRecoveryRequired,
            ]
            .into_iter()
            .find(|event| line == event.code())
            .map(|event| event.code())
        })
        .collect()
}

fn read_safe_panic_marker(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let event = lines.next()?;
    let thread = lines.next()?;
    let line = lines.next()?.strip_prefix("location_line=")?;
    let column = lines.next()?.strip_prefix("location_column=")?;
    if lines.next().is_some()
        || event != "event=panic"
        || thread != "thread=redacted"
        || line.parse::<u32>().is_err()
        || column.parse::<u32>().is_err()
    {
        return None;
    }
    Some(format!(
        "event=panic\nthread=redacted\nlocation_line={line}\nlocation_column={column}\n"
    ))
}

fn map_bundle_file_system_error(_: io::Error) -> DiagnosticsBundleError {
    DiagnosticsBundleError::FileSystem
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticEvent, MinimalDiagnostics, export_diagnostics_bundle, redact_path_for_diagnostics,
        redact_title_for_diagnostics, write_panic_marker,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_test_directory(name: &str) -> PathBuf {
        let number = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("openmanic-{name}-{}-{number}", std::process::id()))
    }

    #[test]
    fn normal_diagnostics_use_only_fixed_codes_and_redacted_values() {
        let data_root = unique_test_directory("diagnostics");
        let diagnostics =
            MinimalDiagnostics::new(&data_root).expect("diagnostics root initializes");
        diagnostics
            .record(DiagnosticEvent::DataRootResolved)
            .expect("fixed diagnostic event writes");
        let log = fs::read_to_string(data_root.join("logs/bootstrap.log"))
            .expect("diagnostic log is readable");

        assert_eq!(log, "bootstrap.data-root.resolved\n");
        assert_eq!(
            redact_title_for_diagnostics("Private window title"),
            "[redacted-title]"
        );
        assert_eq!(
            redact_path_for_diagnostics(PathBuf::from("C:/private/data").as_path()),
            "[redacted-path]"
        );
        assert!(!log.contains("Private window title"));
        assert!(!log.contains("C:/private/data"));
        fs::remove_dir_all(data_root).expect("test diagnostics root is removed");
    }

    #[test]
    fn panic_marker_omits_panic_payload_and_selected_paths() {
        let crash_directory = unique_test_directory("panic-marker");
        fs::create_dir_all(&crash_directory).expect("test crash directory is created");

        write_panic_marker(&crash_directory, 37, 9).expect("minimal panic marker writes");
        let marker = fs::read_to_string(crash_directory.join("panic.marker"))
            .expect("panic marker is readable");

        assert_eq!(
            marker,
            "event=panic\nthread=redacted\nlocation_line=37\nlocation_column=9\n"
        );
        assert!(!marker.contains("title"));
        assert!(!marker.contains("C:/"));
        fs::remove_dir_all(crash_directory).expect("test crash directory is removed");
    }

    #[test]
    fn diagnostics_bundle_has_a_deterministic_manifest_and_excludes_untrusted_text() {
        let data_root = unique_test_directory("diagnostics-bundle-source");
        let destination = unique_test_directory("diagnostics-bundle-destination");
        fs::create_dir_all(data_root.join("logs")).expect("test log directory is created");
        fs::create_dir_all(data_root.join("crash")).expect("test crash directory is created");
        fs::write(
            data_root.join("logs/bootstrap.log"),
            "bootstrap.data-root.resolved\nPrivate window title C:/private/data\n",
        )
        .expect("test log writes");
        fs::write(
            data_root.join("crash/panic.marker"),
            "event=panic\nthread=redacted\nlocation_line=37\nlocation_column=9\nprivate=must-not-export\n",
        )
        .expect("test marker writes");

        let bundle = export_diagnostics_bundle(&data_root, &destination).expect("bundle exports");
        assert_eq!(
            bundle.included_files(),
            &["diagnostics-manifest.txt", "bootstrap.log"]
        );
        assert_eq!(
            fs::read_to_string(destination.join("diagnostics-manifest.txt"))
                .expect("manifest reads"),
            "format_version=1\nprivacy=validated-fixed-diagnostics-only\nfiles=diagnostics-manifest.txt,bootstrap.log\n"
        );
        assert_eq!(
            fs::read_to_string(destination.join("bootstrap.log")).expect("bundle log reads"),
            "bootstrap.data-root.resolved\n"
        );
        assert!(!destination.join("panic.marker").exists());
        let bundle_text = fs::read_to_string(destination.join("bootstrap.log"))
            .expect("bundle log reads");
        assert!(!bundle_text.contains("Private window title"));
        assert!(!bundle_text.contains("C:/private/data"));
        fs::remove_dir_all(data_root).expect("test source is removed");
        fs::remove_dir_all(destination).expect("test destination is removed");
    }

    #[test]
    fn diagnostics_bundle_includes_only_a_valid_redacted_panic_marker() {
        let data_root = unique_test_directory("diagnostics-bundle-panic-source");
        let destination = unique_test_directory("diagnostics-bundle-panic-destination");
        fs::create_dir_all(data_root.join("crash")).expect("test crash directory is created");
        write_panic_marker(&data_root.join("crash"), 37, 9).expect("test marker writes");

        let bundle = export_diagnostics_bundle(&data_root, &destination).expect("bundle exports");
        assert_eq!(
            bundle.included_files(),
            &["diagnostics-manifest.txt", "panic.marker"]
        );
        assert_eq!(
            fs::read_to_string(destination.join("panic.marker")).expect("bundle marker reads"),
            "event=panic\nthread=redacted\nlocation_line=37\nlocation_column=9\n"
        );
        fs::remove_dir_all(data_root).expect("test source is removed");
        fs::remove_dir_all(destination).expect("test destination is removed");
    }
}
