//! Per-user Windows sign-in launch registration.
//!
//! The adapter writes only the current user's `Run` value. It never elevates, schedules a task,
//! or claims a registration succeeded until the registry call has returned successfully.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW,
    RegDeleteValueW, RegSetValueExW,
};
use windows::core::PCWSTR;

const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "OpenManic";

/// The observed result of a requested sign-in launch preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsAutostartStatus {
    /// The Run value was written for the current user.
    Enabled,
    /// The Run value was removed (or was already absent) for the current user.
    Disabled,
}

/// A privacy-safe autostart operation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsAutostartError {
    /// Windows could not open the current-user Run key.
    OpenRunKey,
    /// Windows could not write the OpenManic Run value.
    WriteValue,
    /// Windows could not remove the OpenManic Run value.
    RemoveValue,
}

impl fmt::Display for WindowsAutostartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OpenRunKey => "Windows could not access the current-user sign-in launch setting",
            Self::WriteValue => "Windows could not save the sign-in launch setting",
            Self::RemoveValue => "Windows could not remove the sign-in launch setting",
        })
    }
}

impl Error for WindowsAutostartError {}

/// Adapter for the per-user OpenManic sign-in launch setting.
#[derive(Clone, Debug)]
pub struct WindowsAutostart {
    executable: PathBuf,
}

impl WindowsAutostart {
    /// Creates the adapter for the executable that should be launched at sign-in.
    #[must_use]
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    /// Returns the exact, quoted command written to the Run value.
    #[must_use]
    pub fn command_line(&self) -> String {
        command_line_for(&self.executable)
    }

    /// Enables or disables the current user's sign-in launch setting.
    ///
    /// # Errors
    ///
    /// Returns a stable error when Windows rejects the requested registry operation.
    pub fn set_enabled(
        &self,
        enabled: bool,
    ) -> Result<WindowsAutostartStatus, WindowsAutostartError> {
        let key = open_run_key()?;
        let result = if enabled {
            let command = wide_null(&self.command_line());
            let bytes = wide_bytes(&command);
            let value_name = wide_null(VALUE_NAME);
            // SAFETY: The key and NUL-terminated UTF-16 strings remain valid for this call.
            let status = unsafe {
                RegSetValueExW(
                    key,
                    PCWSTR(value_name.as_ptr()),
                    None,
                    REG_SZ,
                    Some(bytes),
                )
            };
            if status.is_ok() {
                Ok(WindowsAutostartStatus::Enabled)
            } else {
                Err(WindowsAutostartError::WriteValue)
            }
        } else {
            let value_name = wide_null(VALUE_NAME);
            // SAFETY: The key and NUL-terminated value name remain valid for this call.
            let status = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
            if status.is_ok() || status.0 == 2 {
                Ok(WindowsAutostartStatus::Disabled)
            } else {
                Err(WindowsAutostartError::RemoveValue)
            }
        };
        // SAFETY: `key` was returned by `RegCreateKeyW` and is closed exactly once here.
        unsafe { let _ = RegCloseKey(key); };
        result
    }
}

fn open_run_key() -> Result<HKEY, WindowsAutostartError> {
    let subkey = wide_null(RUN_SUBKEY);
    let mut key = HKEY::default();
    // SAFETY: The subkey is NUL-terminated and `key` points to writable storage.
    let status = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), &mut key) };
    if status.is_ok() {
        Ok(key)
    } else {
        Err(WindowsAutostartError::OpenRunKey)
    }
}

fn command_line_for(executable: &Path) -> String {
    let path = executable.to_string_lossy();
    format!("\"{}\" --background", path.replace('"', "\\\""))
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn wide_bytes(value: &[u16]) -> &[u8] {
    // SAFETY: `u16` is contiguous and the resulting byte slice is bounded by its source slice.
    unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), std::mem::size_of_val(value)) }
}

#[cfg(test)]
mod tests {
    use super::{WindowsAutostart, command_line_for};
    use std::path::Path;

    #[test]
    fn sign_in_command_quotes_the_current_executable_and_uses_background_mode() {
        let command = command_line_for(Path::new("C:/Program Files/OpenManic/OpenManic.exe"));
        assert_eq!(command, "\"C:/Program Files/OpenManic/OpenManic.exe\" --background");

        let autostart = WindowsAutostart::new("C:/OpenManic/OpenManic.exe".into());
        assert_eq!(autostart.command_line(), "\"C:/OpenManic/OpenManic.exe\" --background");
    }
}
