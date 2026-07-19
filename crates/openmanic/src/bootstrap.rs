//! Explicit process-startup state before adapter workers or tracking begin.
//!
//! Bootstrap composes CLI input, data-root policy, diagnostics, and first-launch gates. It does
//! not construct storage or platform adapters; the eventual top-level composition passes those
//! capabilities in after this boundary has made data ownership and consent explicit.

use crate::cli::CliOptions;
use crate::data_root::{
    BootstrapLocator, DataRootInputs, DataRootLock, DataRootResolution, DataRootResolutionError,
    DataRootValidator, DirectoryChoiceRequest, ResolvedDataRoot, resolve_data_root,
};
use crate::diagnostics::MinimalDiagnostics;
use std::path::{Path, PathBuf};

/// The user-consent state required before local tracking may start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalTrackingConsent {
    /// First launch has not accepted the local tracking explanation.
    Required,
    /// The user accepted the local tracking explanation.
    Accepted,
}

/// Whether the selected platform adapter is able to supply tracking evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterReadiness {
    /// The platform adapter has not finished readiness checks.
    Pending,
    /// The adapter is ready to supply evidence.
    Ready,
    /// The adapter is unavailable and needs visible remediation.
    Unavailable,
}

/// First-launch and adapter gates that prevent premature tracking claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirstLaunchState {
    consent: LocalTrackingConsent,
    adapter_readiness: AdapterReadiness,
}

impl Default for FirstLaunchState {
    fn default() -> Self {
        Self {
            consent: LocalTrackingConsent::Required,
            adapter_readiness: AdapterReadiness::Pending,
        }
    }
}

impl FirstLaunchState {
    /// Records explicit acceptance of the local tracking explanation.
    pub fn accept_local_tracking_consent(&mut self) {
        self.consent = LocalTrackingConsent::Accepted;
    }

    /// Updates the explicit platform capability/readiness state.
    pub fn set_adapter_readiness(&mut self, readiness: AdapterReadiness) {
        self.adapter_readiness = readiness;
    }

    /// Returns whether tracking can truthfully be started now.
    #[must_use]
    pub const fn may_start_tracking(self) -> bool {
        matches!(self.consent, LocalTrackingConsent::Accepted)
            && matches!(self.adapter_readiness, AdapterReadiness::Ready)
    }

    /// Returns the current consent gate for presentation or persistence wiring.
    #[must_use]
    pub const fn consent(self) -> LocalTrackingConsent {
        self.consent
    }

    /// Returns the current platform readiness gate for presentation or recovery wiring.
    #[must_use]
    pub const fn adapter_readiness(self) -> AdapterReadiness {
        self.adapter_readiness
    }
}

/// The bootstrap result before storage and worker composition begins.
#[derive(Debug)]
pub enum BootstrapDisposition {
    /// The root is validated, exclusively locked, and diagnostics are ready.
    Ready(BootstrapState),
    /// A directory chooser must run before storage opens or tracking can start.
    DirectoryChoiceRequired(DirectoryChoiceRequest),
}

/// Owned bootstrap resources that stay alive until coordinated shutdown.
#[derive(Debug)]
pub struct BootstrapState {
    data_root: ResolvedDataRoot,
    data_root_lock: DataRootLock,
    diagnostics: MinimalDiagnostics,
    first_launch: FirstLaunchState,
}

impl BootstrapState {
    /// Returns the selected and validated data root.
    #[must_use]
    pub fn data_root(&self) -> &ResolvedDataRoot {
        &self.data_root
    }

    /// Returns minimal local diagnostics initialized before workers start.
    #[must_use]
    pub fn diagnostics(&self) -> &MinimalDiagnostics {
        &self.diagnostics
    }

    /// Returns the consent/readiness state that controls tracking start.
    #[must_use]
    pub const fn first_launch(&self) -> FirstLaunchState {
        self.first_launch
    }

    /// Returns mutable first-launch state for explicit UI confirmation and adapter wiring.
    pub fn first_launch_mut(&mut self) -> &mut FirstLaunchState {
        &mut self.first_launch
    }

    /// Returns the held writer lock for lifecycle composition.
    #[must_use]
    pub fn data_root_lock(&self) -> &DataRootLock {
        &self.data_root_lock
    }
}

/// Failure during the limited pre-worker bootstrap sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapError {
    /// An explicit data-root override was invalid and requires recovery.
    DataRootResolution(DataRootResolutionError),
    /// The selected root could not be exclusively locked.
    DataRootLocked,
    /// Minimal local diagnostics could not be initialized.
    DiagnosticsUnavailable,
}

impl BootstrapError {
    /// Returns a stable diagnostic code without user paths or arbitrary text.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DataRootResolution(error) => error.code(),
            Self::DataRootLocked => "bootstrap.data-root.locked",
            Self::DiagnosticsUnavailable => "bootstrap.diagnostics.unavailable",
        }
    }
}

/// Resolves, validates, locks, and initializes diagnostics for process startup.
///
/// `artifact_directory` must identify the actual release artifact directory rather than a working
/// directory. The returned state still has first-launch consent required and adapter readiness
/// pending, so callers cannot truthfully claim tracking before both later prerequisites complete.
///
/// # Errors
///
/// Returns [`BootstrapError`] for an invalid explicit override, writer-lock conflict, or minimal
/// diagnostics failure. It never falls back to a new store after an invalid explicit override.
pub fn bootstrap<V>(
    cli: &CliOptions,
    environment_data_root: Option<PathBuf>,
    locator: Option<BootstrapLocator>,
    artifact_directory: &Path,
    validator: &V,
) -> Result<BootstrapDisposition, BootstrapError>
where
    V: DataRootValidator,
{
    let inputs = DataRootInputs {
        command_line: cli.data_dir_override().cloned(),
        environment: environment_data_root,
        locator,
    };
    let resolution = resolve_data_root(&inputs, artifact_directory, validator)
        .map_err(BootstrapError::DataRootResolution)?;
    let data_root = match resolution {
        DataRootResolution::Ready(data_root) => data_root,
        DataRootResolution::DirectoryChoiceRequired(request) => {
            return Ok(BootstrapDisposition::DirectoryChoiceRequired(request));
        }
    };
    let data_root_lock =
        DataRootLock::acquire(data_root.path()).map_err(|_| BootstrapError::DataRootLocked)?;
    let diagnostics = MinimalDiagnostics::new(data_root.path())
        .map_err(|_| BootstrapError::DiagnosticsUnavailable)?;
    diagnostics
        .install_panic_hook()
        .map_err(|_| BootstrapError::DiagnosticsUnavailable)?;
    Ok(BootstrapDisposition::Ready(BootstrapState {
        data_root,
        data_root_lock,
        diagnostics,
        first_launch: FirstLaunchState::default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterReadiness, BootstrapDisposition, FirstLaunchState, LocalTrackingConsent, bootstrap,
    };
    use crate::cli::parse_cli_arguments;
    use crate::data_root::{DataRootValidationError, DataRootValidator, DirectoryChoiceReason};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    struct ValidatingFilesystem;

    impl DataRootValidator for ValidatingFilesystem {
        fn validate(&self, _: &Path) -> Result<(), DataRootValidationError> {
            Ok(())
        }
    }

    struct RejectingFilesystem;

    impl DataRootValidator for RejectingFilesystem {
        fn validate(&self, _: &Path) -> Result<(), DataRootValidationError> {
            Err(DataRootValidationError::NetworkShare)
        }
    }

    #[test]
    fn tracking_requires_both_explicit_consent_and_adapter_readiness() {
        let mut state = FirstLaunchState::default();

        assert_eq!(state.consent(), LocalTrackingConsent::Required);
        assert_eq!(state.adapter_readiness(), AdapterReadiness::Pending);
        assert!(!state.may_start_tracking());
        state.accept_local_tracking_consent();
        assert!(!state.may_start_tracking());
        state.set_adapter_readiness(AdapterReadiness::Ready);
        assert!(state.may_start_tracking());
        state.set_adapter_readiness(AdapterReadiness::Unavailable);
        assert!(!state.may_start_tracking());
    }

    #[test]
    fn bootstrap_preserves_directory_choice_before_tracking() {
        let cli = parse_cli_arguments(Vec::<OsString>::new()).expect("empty CLI is valid");
        let disposition = bootstrap(
            &cli,
            None,
            None,
            Path::new("artifact"),
            &RejectingFilesystem,
        )
        .expect("unavailable artifact root needs a chooser");

        assert!(matches!(
            disposition,
            BootstrapDisposition::DirectoryChoiceRequired(request)
                if request.reason() == DirectoryChoiceReason::ArtifactDirectoryUnavailable
        ));
    }

    #[test]
    fn bootstrap_accepts_a_valid_explicit_root_before_later_wiring() {
        let root = std::env::temp_dir().join(format!("openmanic-bootstrap-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("test bootstrap root is created");
        let argument = OsString::from(format!("--data-dir={}", root.display()));
        let cli = parse_cli_arguments(vec![argument]).expect("valid explicit root parses");
        let disposition = bootstrap(
            &cli,
            Some(PathBuf::from("ignored-environment-root")),
            None,
            Path::new("ignored-artifact"),
            &ValidatingFilesystem,
        );

        assert!(matches!(disposition, Ok(BootstrapDisposition::Ready(_))));
        drop(disposition);
        let _ = std::fs::remove_dir_all(root);
    }
}
