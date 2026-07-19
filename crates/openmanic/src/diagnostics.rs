//! Minimal local diagnostics that preserve privacy during process bootstrap.
//!
//! The bootstrap boundary records only fixed codes and redacted fields. Detailed diagnostics are a
//! later opt-in capability; ordinary startup output never contains window titles or selected paths.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic;
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticEvent, MinimalDiagnostics, redact_path_for_diagnostics,
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
}
