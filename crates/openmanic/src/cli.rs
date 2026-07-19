//! Minimal command-line input accepted before data-root resolution.
//!
//! The parser deliberately accepts only bootstrap arguments. It does not select services or
//! application behavior, keeping startup input explicit and deterministic.

use std::ffi::OsString;
use std::path::PathBuf;

/// Bootstrap command-line options that do not require opening application storage.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CliOptions {
    data_dir_override: Option<PathBuf>,
    background: bool,
}

impl CliOptions {
    /// Returns the explicit data-root override, when one was supplied.
    #[must_use]
    pub fn data_dir_override(&self) -> Option<&PathBuf> {
        self.data_dir_override.as_ref()
    }

    /// Returns whether the process was started for its background launch path.
    #[must_use]
    pub const fn is_background(&self) -> bool {
        self.background
    }
}

/// A privacy-safe command-line parse error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliError {
    /// `--data-dir` did not have a following path.
    MissingDataDirectory,
    /// More than one data-root override was supplied.
    DuplicateDataDirectory,
    /// An unsupported bootstrap argument was supplied.
    UnsupportedArgument,
}

impl CliError {
    /// Returns the stable diagnostic code for this failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingDataDirectory => "bootstrap.cli.missing-data-dir",
            Self::DuplicateDataDirectory => "bootstrap.cli.duplicate-data-dir",
            Self::UnsupportedArgument => "bootstrap.cli.unsupported-argument",
        }
    }

    /// Returns a user-safe explanation without echoing arbitrary argument text.
    #[must_use]
    pub const fn safe_summary(&self) -> &'static str {
        match self {
            Self::MissingDataDirectory => "The data-directory option needs a directory.",
            Self::DuplicateDataDirectory => "Only one data-directory option may be supplied.",
            Self::UnsupportedArgument => "An unsupported startup option was supplied.",
        }
    }
}

/// Parses bootstrap arguments excluding the executable name.
///
/// # Errors
///
/// Returns [`CliError`] when an option is malformed, duplicated, or unsupported. Error values do
/// not include arbitrary argument text so they are safe to report through ordinary diagnostics.
pub fn parse_cli_arguments<I>(arguments: I) -> Result<CliOptions, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut options = CliOptions::default();
    let mut pending_data_directory = false;

    for argument in arguments {
        if pending_data_directory {
            set_data_directory(&mut options, PathBuf::from(argument))?;
            pending_data_directory = false;
            continue;
        }

        if argument == "--data-dir" {
            pending_data_directory = true;
        } else if let Some(value) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--data-dir="))
        {
            if value.is_empty() {
                return Err(CliError::MissingDataDirectory);
            }
            set_data_directory(&mut options, PathBuf::from(value))?;
        } else if argument == "--background" {
            options.background = true;
        } else {
            return Err(CliError::UnsupportedArgument);
        }
    }

    if pending_data_directory {
        return Err(CliError::MissingDataDirectory);
    }

    Ok(options)
}

/// Parses bootstrap arguments from the process environment.
///
/// # Errors
///
/// Returns [`CliError`] for unsupported or malformed bootstrap options.
pub fn parse_process_cli() -> Result<CliOptions, CliError> {
    parse_cli_arguments(std::env::args_os().skip(1))
}

fn set_data_directory(options: &mut CliOptions, data_directory: PathBuf) -> Result<(), CliError> {
    if options.data_dir_override.is_some() {
        return Err(CliError::DuplicateDataDirectory);
    }
    options.data_dir_override = Some(data_directory);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CliError, parse_cli_arguments};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn accepts_data_directory_and_background_flag() {
        let options = parse_cli_arguments(arguments(&["--data-dir", "C:/data", "--background"]))
            .expect("valid bootstrap arguments parse");

        assert_eq!(options.data_dir_override(), Some(&PathBuf::from("C:/data")));
        assert!(options.is_background());
    }

    #[test]
    fn rejects_missing_data_directory() {
        let error = parse_cli_arguments(arguments(&["--data-dir"]))
            .expect_err("missing data directory must be rejected");

        assert_eq!(error, CliError::MissingDataDirectory);
    }

    #[test]
    fn rejects_duplicate_data_directory() {
        let error = parse_cli_arguments(arguments(&["--data-dir", "first", "--data-dir=second"]))
            .expect_err("duplicate override must be rejected");

        assert_eq!(error, CliError::DuplicateDataDirectory);
    }
}
