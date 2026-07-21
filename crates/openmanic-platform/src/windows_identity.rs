//! Private Windows process and application identity support.
//!
//! This module deliberately keeps native process handles and raw window values at the adapter
//! edge. A foreground window first becomes a `(PID, creation time)` process instance; only then
//! can a cached packaged AUMID or executable-path candidate be reused. In particular, neither a
//! PID, an HWND, a caption, nor an executable filename is ever a stable application identity.

#![expect(
    dead_code,
    reason = "resolver details remain private until later catalog presentation and diagnostics work"
)]

use std::collections::HashMap;

/// Resolves Windows application candidates while defending its cache against PID reuse.
///
/// This remains crate-private until the primary composition task connects it to the foreground
/// control loop. Keeping it separate means that an incomplete resolver cannot accidentally turn
/// the current explicit identity degradation into fabricated foreground attribution.
#[derive(Debug, Default)]
pub(crate) struct WindowsIdentityResolver {
    resolved_by_process_instance: HashMap<ProcessInstanceKey, ResolvedApplication>,
}

impl WindowsIdentityResolver {
    /// Resolves a pre-inspected process, using a cache key that includes its creation time.
    ///
    /// The inspection closure is intentionally deferred until after the cache lookup. Callers
    /// must obtain the creation time from the same process handle used for the eventual inspection.
    pub(crate) fn resolve_for_process<F>(
        &mut self,
        process_instance: ProcessInstanceKey,
        inspect: F,
    ) -> ApplicationIdentityResolution
    where
        F: FnOnce() -> ProcessApplicationInspection,
    {
        if let Some(resolved) = self.resolved_by_process_instance.get(&process_instance) {
            return ApplicationIdentityResolution::Resolved(resolved.clone());
        }

        let resolution = match inspect() {
            ProcessApplicationInspection::Packaged {
                aumid,
                package_family,
            } => ApplicationIdentityResolution::Resolved(ResolvedApplication {
                process_instance,
                identity: StableApplicationIdentity::Packaged {
                    aumid,
                    package_family,
                },
                display_path: None,
                confidence: IdentityConfidence::High,
            }),
            ProcessApplicationInspection::ExecutablePath { full_path } => {
                let Some(executable_path) =
                    WindowsExecutablePath::from_full_process_path(full_path)
                else {
                    return ApplicationIdentityResolution::Unresolved {
                        process_instance: Some(process_instance),
                        reason: IdentityUncertainty::InvalidExecutablePath,
                    };
                };
                let display_path = executable_path.display_path().to_owned();
                ApplicationIdentityResolution::Resolved(ResolvedApplication {
                    process_instance,
                    identity: StableApplicationIdentity::ExecutablePath { executable_path },
                    display_path: Some(display_path),
                    confidence: IdentityConfidence::High,
                })
            }
            ProcessApplicationInspection::HostedPackagedApp { package_family } => {
                ApplicationIdentityResolution::Unresolved {
                    process_instance: Some(process_instance),
                    reason: IdentityUncertainty::HostedPackagedApp { package_family },
                }
            }
            ProcessApplicationInspection::Unresolved(reason) => {
                ApplicationIdentityResolution::Unresolved {
                    process_instance: Some(process_instance),
                    reason,
                }
            }
        };

        if let ApplicationIdentityResolution::Resolved(resolved) = &resolution {
            self.resolved_by_process_instance
                .insert(process_instance, resolved.clone());
        }
        resolution
    }

    /// Resolves one private raw Windows window value without allowing it to leave this module.
    ///
    /// Ordinary operating-system failures become [`ApplicationIdentityResolution::Unresolved`].
    /// This method does not expose native error values or request additional privilege.
    #[cfg(windows)]
    pub(crate) fn resolve_window(&mut self, window_value: isize) -> ApplicationIdentityResolution {
        use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::GetWindowThreadProcessId};

        if window_value == 0 {
            return ApplicationIdentityResolution::unresolved(
                IdentityUncertainty::WindowUnavailable,
            );
        }

        let mut process_id = 0_u32;
        // SAFETY: The opaque value originates from the adapter's private Win32 ingress. Windows
        // only writes the supplied `u32` PID and does not retain its pointer.
        let thread_id = unsafe {
            GetWindowThreadProcessId(HWND(window_value as *mut _), Some(&raw mut process_id))
        };
        if thread_id == 0 || process_id == 0 {
            return ApplicationIdentityResolution::unresolved(
                IdentityUncertainty::WindowUnavailable,
            );
        }

        let process = match OpenProcessHandle::open(process_id) {
            Ok(process) => process,
            Err(reason) => return ApplicationIdentityResolution::unresolved(reason),
        };
        let process_instance = match process.process_instance_key(process_id) {
            Ok(process_instance) => process_instance,
            Err(reason) => {
                return ApplicationIdentityResolution::Unresolved {
                    process_instance: None,
                    reason,
                };
            }
        };

        self.resolve_for_process(process_instance, || process.inspect_application())
    }
}

/// A Windows process identity that cannot be confused with a later PID reuse.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProcessInstanceKey {
    process_id: u32,
    creation_time_filetime: u64,
}

impl ProcessInstanceKey {
    /// Creates a process-instance cache key from a PID and Windows FILETIME creation value.
    #[must_use]
    pub(crate) const fn new(process_id: u32, creation_time_filetime: u64) -> Self {
        Self {
            process_id,
            creation_time_filetime,
        }
    }

    /// Returns the process identifier for diagnostics scoped to this exact instance.
    #[must_use]
    pub(crate) const fn process_id(self) -> u32 {
        self.process_id
    }

    /// Returns the Windows FILETIME creation value that prevents PID-only cache reuse.
    #[must_use]
    pub(crate) const fn creation_time_filetime(self) -> u64 {
        self.creation_time_filetime
    }
}

/// The outcome of one Windows application-identity lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ApplicationIdentityResolution {
    /// A stable candidate is available for the exact observed process instance.
    Resolved(ResolvedApplication),
    /// No stable candidate can be stated honestly for the observed window or process.
    Unresolved {
        /// The resolved process instance when Windows allowed it to be read.
        process_instance: Option<ProcessInstanceKey>,
        /// The explicit reason stable application attribution is uncertain.
        reason: IdentityUncertainty,
    },
}

impl ApplicationIdentityResolution {
    const fn unresolved(reason: IdentityUncertainty) -> Self {
        Self::Unresolved {
            process_instance: None,
            reason,
        }
    }
}

/// A stable Windows identity candidate and its separately retained display information.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedApplication {
    process_instance: ProcessInstanceKey,
    identity: StableApplicationIdentity,
    display_path: Option<String>,
    confidence: IdentityConfidence,
}

impl ResolvedApplication {
    /// Returns the exact process instance that supplied this candidate.
    #[must_use]
    pub(crate) const fn process_instance(&self) -> ProcessInstanceKey {
        self.process_instance
    }

    /// Returns the stable application candidate, never a title, filename, HWND, or PID.
    #[must_use]
    pub(crate) const fn identity(&self) -> &StableApplicationIdentity {
        &self.identity
    }

    /// Returns the original full executable path for display or privacy-filtered diagnostics.
    ///
    /// The path is deliberately not used as an application label or a replacement for the
    /// normalized candidate stored in [`StableApplicationIdentity`].
    #[must_use]
    pub(crate) fn display_path(&self) -> Option<&str> {
        self.display_path.as_deref()
    }

    /// Returns the confidence appropriate for this candidate.
    #[must_use]
    pub(crate) const fn confidence(&self) -> IdentityConfidence {
        self.confidence
    }
}

/// The stable candidate forms that an application-layer identity service may map to a record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StableApplicationIdentity {
    /// A packaged/application-mode candidate whose AUMID is the stable key.
    Packaged {
        /// The explicit process AUMID; package family remains metadata rather than a duplicate key.
        aumid: String,
        /// Package family metadata when Windows made it available.
        package_family: Option<String>,
    },
    /// An ordinary unpackaged desktop candidate identified by its full executable path.
    ExecutablePath {
        /// Full process-image path with Windows-aware equality, kept separate from presentation.
        executable_path: WindowsExecutablePath,
    },
}

/// A full executable path that preserves its displayed spelling and compares using Windows rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowsExecutablePath {
    identity_path: String,
    display_path: String,
}

impl WindowsExecutablePath {
    fn from_full_process_path(full_path: String) -> Option<Self> {
        let normalized_path = normalize_full_windows_process_path(&full_path)?;
        Some(Self {
            identity_path: normalized_path,
            display_path: full_path,
        })
    }

    /// Returns the full process-image path used as the stable desktop candidate.
    #[must_use]
    pub(crate) fn identity_path(&self) -> &str {
        &self.identity_path
    }

    /// Returns the original spelling returned by `QueryFullProcessImageNameW`.
    #[must_use]
    pub(crate) fn display_path(&self) -> &str {
        &self.display_path
    }

    /// Compares two full executable paths with Windows ordinal case-insensitive semantics.
    ///
    /// Windows filesystem case behavior must not be approximated by Unicode `to_lowercase`.
    /// This function calls `CompareStringOrdinal` on Windows and is intentionally unavailable on
    /// other platforms rather than silently changing the comparison contract.
    #[cfg(windows)]
    #[must_use]
    pub(crate) fn is_same_windows_path_as(&self, other: &Self) -> bool {
        use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

        let left: Vec<u16> = self.identity_path.encode_utf16().collect();
        let right: Vec<u16> = other.identity_path.encode_utf16().collect();
        // SAFETY: The Windows binding accepts borrowed UTF-16 slices for the duration of this
        // call. It does not retain either pointer and ordinal comparison has no side effects.
        unsafe { CompareStringOrdinal(&left, &right, true) == CSTR_EQUAL }
    }
}

/// The confidence carried with an identity candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdentityConfidence {
    /// Windows returned a stable AUMID or full executable image path for this process instance.
    High,
}

/// An honest reason that a foreground process cannot become a stable application candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IdentityUncertainty {
    /// The foreground handle disappeared or never had a process before resolution.
    WindowUnavailable,
    /// Windows refused process-query access while the app stayed unelevated.
    AccessDenied,
    /// A deterministic probe identified a protected process that must not be attributed.
    ///
    /// The live Windows source does not speculate that every access denial is protection; it
    /// reports [`Self::AccessDenied`] unless a supported probe has affirmative evidence.
    ProtectedProcess,
    /// The observation is the system/idle pseudo-process, which has no user application identity.
    SystemProcess,
    /// The target process exited before its creation time or identity could be read.
    ProcessExited,
    /// A packaged host had package metadata but no explicit AUMID, so app attribution is unclear.
    HostedPackagedApp {
        /// Package metadata retained only to explain why no path fallback was used.
        package_family: Option<String>,
    },
    /// Windows returned malformed or overlong AUMID/package metadata.
    InvalidPackagedIdentity,
    /// Windows returned an unusable full process-image path.
    InvalidExecutablePath,
    /// A supported query failed without a safe, more-specific classification.
    LookupFailed,
}

/// A deterministic process-inspection result used by the cache and its fakes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessApplicationInspection {
    /// An explicit AUMID takes precedence over every executable-path candidate.
    Packaged {
        /// The explicit AUMID used as the stable packaged candidate.
        aumid: String,
        /// Package family metadata when Windows supplied it.
        package_family: Option<String>,
    },
    /// An unpackaged process with a full executable image path.
    ExecutablePath {
        /// The full image path returned by Windows, including its original display spelling.
        full_path: String,
    },
    /// A packaged host lacks a specific AUMID, so the hosted application is ambiguous.
    HostedPackagedApp {
        /// Package family metadata, which is not promoted into a stable application key.
        package_family: Option<String>,
    },
    /// A process could not yield a safe stable identity.
    Unresolved(IdentityUncertainty),
}

fn normalize_full_windows_process_path(full_path: &str) -> Option<String> {
    if full_path.is_empty() || full_path.contains('\0') || !is_full_windows_path(full_path) {
        return None;
    }

    // `QueryFullProcessImageNameW(PROCESS_NAME_WIN32)` already returns the full Win32 image path.
    // Preserve its Unicode spelling; equality is delegated to CompareStringOrdinal above instead
    // of performing lossy or locale-dependent Unicode case conversion here.
    //
    // Collapse version-bearing install directories so that an application keeps one stable
    // identity across auto-updates (Squirrel/Electron apps install each version into a folder
    // such as `app-0.11.4`). Only the version segment is rewritten, so unrelated applications
    // whose remaining path differs never collide.
    let replaced = full_path.replace('/', "\\");
    let normalized = replaced
        .split('\\')
        .map(collapse_versioned_path_segment)
        .collect::<Vec<_>>()
        .join("\\");
    Some(normalized)
}

/// Rewrites a single path segment that encodes an application version to a fixed token.
///
/// Recognizes the Squirrel/Electron `app-<version>` layout (for example `app-0.11.4`) and bare
/// version directories (for example `1.2.3`). Every other segment, including executable file
/// names and ordinary folders, is returned unchanged.
fn collapse_versioned_path_segment(segment: &str) -> &str {
    if is_squirrel_versioned_segment(segment) {
        "app-<version>"
    } else if is_bare_version_segment(segment) {
        "<version>"
    } else {
        segment
    }
}

/// Returns whether a segment is the Squirrel `app-<version>` install-directory form.
fn is_squirrel_versioned_segment(segment: &str) -> bool {
    let prefix = b"app-";
    let bytes = segment.as_bytes();
    if bytes.len() <= prefix.len() || !bytes[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return false;
    }
    is_bare_version_segment(&segment[prefix.len()..])
}

/// Returns whether a segment is a bare dotted version such as `1.2.3` or `0.11.4.567`.
fn is_bare_version_segment(segment: &str) -> bool {
    let mut components = 0_u32;
    for component in segment.split('.') {
        if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        components += 1;
    }
    components >= 2
}

fn is_full_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive_rooted = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc_or_extended = path.starts_with("\\\\") || path.starts_with("//");
    drive_rooted || unc_or_extended
}

#[cfg(windows)]
const MAX_PROCESS_IDENTITY_UTF16: u32 = 32_768;

#[cfg(windows)]
struct OpenProcessHandle {
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl OpenProcessHandle {
    fn open(process_id: u32) -> Result<Self, IdentityUncertainty> {
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

        if process_id == 0 {
            return Err(IdentityUncertainty::SystemProcess);
        }
        // SAFETY: We ask only for documented limited query access, do not inherit the handle,
        // retain it in this private RAII wrapper, and never elevate or adjust process privileges.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
            .map_err(|error| classify_process_query_error(&error))?;
        Ok(Self { handle })
    }

    fn process_instance_key(
        &self,
        process_id: u32,
    ) -> Result<ProcessInstanceKey, IdentityUncertainty> {
        use windows::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: Every pointer refers to initialized stack storage for the call only. The owned
        // process handle remains valid through the query and Windows retains none of the pointers.
        unsafe {
            GetProcessTimes(
                self.handle,
                &raw mut creation,
                &raw mut exit,
                &raw mut kernel,
                &raw mut user,
            )
        }
        .map_err(|error| classify_process_query_error(&error))?;
        let creation_time_filetime =
            (u64::from(creation.dwHighDateTime) << u32::BITS) | u64::from(creation.dwLowDateTime);
        Ok(ProcessInstanceKey::new(process_id, creation_time_filetime))
    }

    fn inspect_application(&self) -> ProcessApplicationInspection {
        match read_application_user_model_id(self.handle) {
            NativeStringRead::Value(aumid) => {
                let package_family = match read_package_family_name(self.handle) {
                    NativeStringRead::Value(package_family) => Some(package_family),
                    NativeStringRead::NotAvailable
                    | NativeStringRead::AccessDenied
                    | NativeStringRead::Invalid
                    | NativeStringRead::Failed => None,
                };
                ProcessApplicationInspection::Packaged {
                    aumid,
                    package_family,
                }
            }
            NativeStringRead::NotAvailable => match read_package_family_name(self.handle) {
                NativeStringRead::Value(package_family) => {
                    ProcessApplicationInspection::HostedPackagedApp {
                        package_family: Some(package_family),
                    }
                }
                NativeStringRead::NotAvailable => match read_full_process_image_name(self.handle) {
                    NativeStringRead::Value(full_path) => {
                        ProcessApplicationInspection::ExecutablePath { full_path }
                    }
                    NativeStringRead::AccessDenied => {
                        ProcessApplicationInspection::Unresolved(IdentityUncertainty::AccessDenied)
                    }
                    NativeStringRead::Invalid => ProcessApplicationInspection::Unresolved(
                        IdentityUncertainty::InvalidExecutablePath,
                    ),
                    NativeStringRead::NotAvailable | NativeStringRead::Failed => {
                        ProcessApplicationInspection::Unresolved(IdentityUncertainty::LookupFailed)
                    }
                },
                NativeStringRead::AccessDenied => {
                    ProcessApplicationInspection::Unresolved(IdentityUncertainty::AccessDenied)
                }
                NativeStringRead::Invalid | NativeStringRead::Failed => {
                    ProcessApplicationInspection::Unresolved(IdentityUncertainty::LookupFailed)
                }
            },
            NativeStringRead::AccessDenied => {
                ProcessApplicationInspection::Unresolved(IdentityUncertainty::AccessDenied)
            }
            NativeStringRead::Invalid => ProcessApplicationInspection::Unresolved(
                IdentityUncertainty::InvalidPackagedIdentity,
            ),
            NativeStringRead::Failed => {
                ProcessApplicationInspection::Unresolved(IdentityUncertainty::LookupFailed)
            }
        }
    }
}

#[cfg(windows)]
impl Drop for OpenProcessHandle {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;

        // SAFETY: This RAII wrapper owns the successful `OpenProcess` handle exactly once. The
        // close result cannot change the already-produced observation and must not panic in Drop.
        drop(unsafe { CloseHandle(self.handle) });
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
enum NativeStringRead {
    Value(String),
    NotAvailable,
    AccessDenied,
    Invalid,
    Failed,
}

#[cfg(windows)]
fn read_application_user_model_id(handle: windows::Win32::Foundation::HANDLE) -> NativeStringRead {
    use windows::{
        Win32::{
            Foundation::{
                APPMODEL_ERROR_NO_APPLICATION, APPMODEL_ERROR_NO_PACKAGE,
                ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
            },
            Storage::Packaging::Appx::GetApplicationUserModelId,
        },
        core::PWSTR,
    };

    let mut length = 0_u32;
    // SAFETY: A null output buffer with a valid length pointer is the documented size query. The
    // process handle is owned by the private RAII wrapper and remains valid for this call.
    let first = unsafe { GetApplicationUserModelId(handle, &raw mut length, None) };
    if first == APPMODEL_ERROR_NO_APPLICATION || first == APPMODEL_ERROR_NO_PACKAGE {
        return NativeStringRead::NotAvailable;
    }
    if first != ERROR_INSUFFICIENT_BUFFER || !is_reasonable_windows_string_length(length) {
        return classify_win32_string_error(first);
    }

    let mut buffer = vec![0_u16; length as usize];
    // SAFETY: The vector allocation provides exactly `length` mutable UTF-16 units for the
    // documented second query. Windows writes only within that allocation and retains no pointer.
    let second = unsafe {
        GetApplicationUserModelId(handle, &raw mut length, Some(PWSTR(buffer.as_mut_ptr())))
    };
    if second != ERROR_SUCCESS {
        return classify_win32_string_error(second);
    }
    decode_windows_string(buffer, length)
}

#[cfg(windows)]
fn read_package_family_name(handle: windows::Win32::Foundation::HANDLE) -> NativeStringRead {
    use windows::{
        Win32::{
            Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
            Storage::Packaging::Appx::GetPackageFamilyName,
        },
        core::PWSTR,
    };

    let mut length = 0_u32;
    // SAFETY: A null output buffer with a valid length pointer is the documented size query. The
    // process handle is private, valid for the call, and not retained by Windows.
    let first = unsafe { GetPackageFamilyName(handle, &raw mut length, None) };
    if first == APPMODEL_ERROR_NO_PACKAGE {
        return NativeStringRead::NotAvailable;
    }
    if first != ERROR_INSUFFICIENT_BUFFER || !is_reasonable_windows_string_length(length) {
        return classify_win32_string_error(first);
    }

    let mut buffer = vec![0_u16; length as usize];
    // SAFETY: The allocated buffer has the exact reported capacity and is used only for this
    // synchronous Win32 call; Windows neither reads after return nor retains the pointer.
    let second =
        unsafe { GetPackageFamilyName(handle, &raw mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if second != ERROR_SUCCESS {
        return classify_win32_string_error(second);
    }
    decode_windows_string(buffer, length)
}

#[cfg(windows)]
fn read_full_process_image_name(handle: windows::Win32::Foundation::HANDLE) -> NativeStringRead {
    use windows::{
        Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW},
        core::PWSTR,
    };

    let mut capacity = 512_u32;
    while capacity <= MAX_PROCESS_IDENTITY_UTF16 {
        let mut buffer = vec![0_u16; capacity as usize];
        let mut length = capacity;
        // SAFETY: The private process handle is live. The vector owns `capacity` UTF-16 units for
        // this synchronous call, and the reported size pointer is valid for its duration only.
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &raw mut length,
            )
        };
        match result {
            Ok(()) if length < capacity => return decode_windows_string(buffer, length),
            Ok(()) | Err(_) if capacity == MAX_PROCESS_IDENTITY_UTF16 => {
                return NativeStringRead::Invalid;
            }
            Ok(()) => {
                capacity = capacity.saturating_mul(2).min(MAX_PROCESS_IDENTITY_UTF16);
            }
            Err(error) if is_windows_error(&error, 122) => {
                capacity = capacity.saturating_mul(2).min(MAX_PROCESS_IDENTITY_UTF16);
            }
            Err(error) if is_windows_error(&error, 5) => return NativeStringRead::AccessDenied,
            Err(_) => return NativeStringRead::Failed,
        }
    }
    NativeStringRead::Invalid
}

#[cfg(windows)]
fn decode_windows_string(mut buffer: Vec<u16>, length: u32) -> NativeStringRead {
    let used = usize::try_from(length).ok();
    let Some(used) = used.filter(|used| *used <= buffer.len()) else {
        return NativeStringRead::Invalid;
    };
    buffer.truncate(used);
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    let Ok(value) = String::from_utf16(&buffer) else {
        return NativeStringRead::Invalid;
    };
    if value.is_empty() || value.contains('\0') {
        NativeStringRead::Invalid
    } else {
        NativeStringRead::Value(value)
    }
}

#[cfg(windows)]
const fn is_reasonable_windows_string_length(length: u32) -> bool {
    length > 1 && length <= MAX_PROCESS_IDENTITY_UTF16
}

#[cfg(windows)]
fn classify_win32_string_error(error: windows::Win32::Foundation::WIN32_ERROR) -> NativeStringRead {
    use windows::Win32::Foundation::{
        APPMODEL_ERROR_NO_APPLICATION, APPMODEL_ERROR_NO_PACKAGE, ERROR_ACCESS_DENIED,
    };

    if error == APPMODEL_ERROR_NO_APPLICATION || error == APPMODEL_ERROR_NO_PACKAGE {
        NativeStringRead::NotAvailable
    } else if error == ERROR_ACCESS_DENIED {
        NativeStringRead::AccessDenied
    } else {
        NativeStringRead::Failed
    }
}

#[cfg(windows)]
fn classify_process_query_error(error: &windows::core::Error) -> IdentityUncertainty {
    if is_windows_error(error, 5) {
        IdentityUncertainty::AccessDenied
    } else {
        IdentityUncertainty::ProcessExited
    }
}

#[cfg(windows)]
fn is_windows_error(error: &windows::core::Error, code: u32) -> bool {
    error.code() == windows::core::HRESULT::from_win32(code)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    #[cfg(windows)]
    use super::WindowsExecutablePath;
    use super::{
        ApplicationIdentityResolution, IdentityUncertainty, ProcessApplicationInspection,
        ProcessInstanceKey, StableApplicationIdentity, WindowsIdentityResolver,
    };

    #[test]
    fn cache_key_includes_process_creation_time_not_just_pid() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        let first = ProcessInstanceKey::new(42, 10);
        let reused_pid = ProcessInstanceKey::new(42, 11);

        let first_resolution =
            identity_resolver.resolve_for_process(first, || executable("C:\\Apps\\first.exe"));
        let reused_resolution = identity_resolver
            .resolve_for_process(reused_pid, || executable("C:\\Apps\\second.exe"));

        assert!(matches!(
            first_resolution,
            ApplicationIdentityResolution::Resolved(ref resolved)
                if resolved.process_instance() == first
        ));
        assert!(matches!(
            reused_resolution,
            ApplicationIdentityResolution::Resolved(ref resolved)
                if resolved.process_instance() == reused_pid
        ));
        assert_eq!(
            resolved_path(&first_resolution),
            Some("C:\\Apps\\first.exe")
        );
        assert_eq!(
            resolved_path(&reused_resolution),
            Some("C:\\Apps\\second.exe")
        );
    }

    #[test]
    fn same_process_instance_reuses_a_resolved_candidate_without_reinspection() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        let process = ProcessInstanceKey::new(12, 99);
        let inspections = Cell::new(0_u8);

        let first = identity_resolver.resolve_for_process(process, || {
            inspections.set(inspections.get() + 1);
            packaged("Contoso.Reader_123!Main", Some("Contoso.Reader_123"))
        });
        let cached = identity_resolver.resolve_for_process(process, || {
            inspections.set(inspections.get() + 1);
            executable("C:\\should-not-be-read.exe")
        });

        assert_eq!(inspections.get(), 1);
        assert_eq!(first, cached);
        assert!(matches!(
            first,
            ApplicationIdentityResolution::Resolved(ref resolved)
                if matches!(
                    resolved.identity(),
                    StableApplicationIdentity::Packaged { aumid, package_family }
                        if aumid == "Contoso.Reader_123!Main"
                            && package_family.as_deref() == Some("Contoso.Reader_123")
                )
        ));
    }

    #[test]
    fn packaged_aumid_wins_over_executable_path_and_keeps_family_as_metadata() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        let resolution = identity_resolver
            .resolve_for_process(ProcessInstanceKey::new(3, 4), || {
                packaged("Vendor.App_abc!Shell", Some("Vendor.App_abc"))
            });

        let packaged_without_display_path = matches!(
            &resolution,
            ApplicationIdentityResolution::Resolved(resolved)
                if resolved.display_path().is_none()
                    && matches!(
                        resolved.identity(),
                        StableApplicationIdentity::Packaged { aumid, package_family }
                            if aumid == "Vendor.App_abc!Shell"
                                && package_family.as_deref() == Some("Vendor.App_abc")
                    )
        );
        assert!(packaged_without_display_path);
    }

    #[test]
    fn packaged_host_without_aumid_never_falls_back_to_its_host_executable() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        let resolution =
            identity_resolver.resolve_for_process(ProcessInstanceKey::new(8, 9), || {
                ProcessApplicationInspection::HostedPackagedApp {
                    package_family: Some("Vendor.Host_abc".to_owned()),
                }
            });

        assert_eq!(
            resolution,
            ApplicationIdentityResolution::Unresolved {
                process_instance: Some(ProcessInstanceKey::new(8, 9)),
                reason: IdentityUncertainty::HostedPackagedApp {
                    package_family: Some("Vendor.Host_abc".to_owned()),
                },
            }
        );
    }

    #[test]
    fn access_denied_and_protected_processes_stay_explicitly_unresolved() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        for reason in [
            IdentityUncertainty::AccessDenied,
            IdentityUncertainty::ProtectedProcess,
            IdentityUncertainty::SystemProcess,
        ] {
            let resolution = identity_resolver
                .resolve_for_process(ProcessInstanceKey::new(55, 66), || {
                    ProcessApplicationInspection::Unresolved(reason.clone())
                });
            assert!(matches!(
                resolution,
                ApplicationIdentityResolution::Unresolved { reason: observed, .. } if observed == reason
            ));
        }
    }

    #[test]
    fn title_or_filename_alone_cannot_become_an_identity() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        let title_like = identity_resolver
            .resolve_for_process(ProcessInstanceKey::new(99, 1), || executable("reader.exe"));

        assert_eq!(
            title_like,
            ApplicationIdentityResolution::Unresolved {
                process_instance: Some(ProcessInstanceKey::new(99, 1)),
                reason: IdentityUncertainty::InvalidExecutablePath,
            }
        );
    }

    #[test]
    fn invalid_or_empty_executable_path_remains_unresolved() {
        let mut identity_resolver = WindowsIdentityResolver::default();
        let resolution =
            identity_resolver.resolve_for_process(ProcessInstanceKey::new(7, 7), || executable(""));

        assert_eq!(
            resolution,
            ApplicationIdentityResolution::Unresolved {
                process_instance: Some(ProcessInstanceKey::new(7, 7)),
                reason: IdentityUncertainty::InvalidExecutablePath,
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn executable_paths_use_windows_ordinal_comparison_without_unicode_lowercasing() {
        let first =
            WindowsExecutablePath::from_full_process_path("C:/Apps/\u{00c5}pp.exe".to_owned())
                .expect("the deterministic full process path is valid");
        let second =
            WindowsExecutablePath::from_full_process_path("c:\\apps\\\u{00e5}PP.EXE".to_owned())
                .expect("the deterministic full process path is valid");

        assert!(first.is_same_windows_path_as(&second));
        assert_eq!(first.display_path(), "C:/Apps/\u{00c5}pp.exe");
        assert_eq!(first.identity_path(), "C:\\Apps\\\u{00c5}pp.exe");
    }

    #[test]
    fn versioned_squirrel_install_directories_share_one_identity_path() {
        let older = WindowsExecutablePath::from_full_process_path(
            "C:\\Users\\me\\AppData\\Local\\Claude\\app-0.11.4\\claude.exe".to_owned(),
        )
        .expect("the versioned path is valid");
        let newer = WindowsExecutablePath::from_full_process_path(
            "C:\\Users\\me\\AppData\\Local\\Claude\\app-0.11.5\\claude.exe".to_owned(),
        )
        .expect("the versioned path is valid");

        assert_eq!(older.identity_path(), newer.identity_path());
        assert_eq!(
            older.identity_path(),
            "C:\\Users\\me\\AppData\\Local\\Claude\\app-<version>\\claude.exe"
        );
        // The original spelling is retained for display and metadata resolution.
        assert_eq!(
            older.display_path(),
            "C:\\Users\\me\\AppData\\Local\\Claude\\app-0.11.4\\claude.exe"
        );
    }

    #[test]
    fn bare_version_directories_collapse_but_ordinary_segments_do_not() {
        let path = WindowsExecutablePath::from_full_process_path(
            "C:\\Program Files\\Vendor\\2.3.1\\bin\\tool.exe".to_owned(),
        )
        .expect("the versioned path is valid");
        assert_eq!(
            path.identity_path(),
            "C:\\Program Files\\Vendor\\<version>\\bin\\tool.exe"
        );

        // Non-version segments (a lone number, a name with a dotted filename, `app-beta`) are
        // left untouched so unrelated applications never merge.
        let untouched = WindowsExecutablePath::from_full_process_path(
            "C:\\Games\\app-beta\\2024\\game.v2.exe".to_owned(),
        )
        .expect("the path is valid");
        assert_eq!(
            untouched.identity_path(),
            "C:\\Games\\app-beta\\2024\\game.v2.exe"
        );
    }

    fn executable(full_path: &str) -> ProcessApplicationInspection {
        ProcessApplicationInspection::ExecutablePath {
            full_path: full_path.to_owned(),
        }
    }

    fn packaged(aumid: &str, package_family: Option<&str>) -> ProcessApplicationInspection {
        ProcessApplicationInspection::Packaged {
            aumid: aumid.to_owned(),
            package_family: package_family.map(str::to_owned),
        }
    }

    fn resolved_path(resolution: &ApplicationIdentityResolution) -> Option<&str> {
        let ApplicationIdentityResolution::Resolved(resolved) = resolution else {
            return None;
        };
        match resolved.identity() {
            StableApplicationIdentity::ExecutablePath { executable_path } => {
                Some(executable_path.identity_path())
            }
            StableApplicationIdentity::Packaged { .. } => None,
        }
    }
}
