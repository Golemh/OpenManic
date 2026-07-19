//! Dependency-policy tool validation and execution.

use std::{io::ErrorKind, path::Path};

use crate::command::{CommandSpec, Failure, run_command};

const CARGO_DENY_VERSION: &str = "0.20.2";
const INSTALL_COMMAND: &str = "cargo install --locked cargo-deny --version 0.20.2";

pub(crate) fn run(repository: &Path) -> Result<(), Failure> {
    let version_command = CommandSpec::new("cargo-deny", ["--version"]);
    println!("$ {}", version_command.display());
    let output = version_command
        .process(repository)
        .output()
        .map_err(|error| missing_or_unusable_tool(&version_command, &error))?;
    if !output.status.success() {
        return Err(Failure::check(format!(
            "cargo-deny version check failed; install the required version with `{INSTALL_COMMAND}`"
        )));
    }

    let reported = String::from_utf8_lossy(&output.stdout);
    validate_version(&reported)?;
    run_command(repository, &policy_command())
}

fn missing_or_unusable_tool(command: &CommandSpec, error: &std::io::Error) -> Failure {
    if error.kind() == ErrorKind::NotFound {
        Failure::check(format!(
            "cargo-deny {CARGO_DENY_VERSION} is required; install it with `{INSTALL_COMMAND}`"
        ))
    } else {
        Failure::spawn(command, error)
    }
}

fn validate_version(reported: &str) -> Result<(), Failure> {
    let found = reported.split_whitespace().nth(1);
    if found == Some(CARGO_DENY_VERSION) {
        Ok(())
    } else {
        Err(Failure::check(format!(
            "cargo-deny {CARGO_DENY_VERSION} is required, but found `{}`; install the required version with `{INSTALL_COMMAND}`",
            reported.trim()
        )))
    }
}

fn policy_command() -> CommandSpec {
    CommandSpec::new("cargo-deny", ["check", "advisories", "bans", "sources"])
}

#[cfg(test)]
mod tests {
    use super::{policy_command, validate_version};

    #[test]
    fn accepts_only_the_pinned_cargo_deny_version() {
        assert!(validate_version("cargo-deny 0.20.2\n").is_ok());
        let error = validate_version("cargo-deny 0.20.1\n").expect_err("old version must fail");
        assert!(error.to_string().contains("--version 0.20.2"));
    }

    #[test]
    fn dependency_policy_avoids_incompatible_all_features() {
        let command = policy_command().display();
        assert_eq!(command, "cargo-deny check advisories bans sources");
        assert!(!command.contains("--all-features"));
    }
}
