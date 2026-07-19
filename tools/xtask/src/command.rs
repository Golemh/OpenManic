//! Child-process descriptions and execution.

use std::{fmt, io, path::Path, process::Command};

#[derive(Debug)]
pub(crate) struct Failure {
    message: String,
    exit_code: u8,
}

impl Failure {
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::new(message, 2)
    }

    pub(crate) fn check(message: impl Into<String>) -> Self {
        Self::new(message, 1)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(message, 1)
    }

    pub(crate) fn process(command: &CommandSpec, status_code: Option<i32>) -> Self {
        let exit_code = status_code
            .and_then(|code| u8::try_from(code).ok())
            .filter(|code| *code != 0)
            .unwrap_or(1);
        Self::new(
            format!(
                "command failed with status {status_code:?}: `{}`",
                command.display()
            ),
            exit_code,
        )
    }

    pub(crate) fn spawn(command: &CommandSpec, error: &io::Error) -> Self {
        Self::new(
            format!("could not start `{}`: {error}", command.display()),
            1,
        )
    }

    fn new(message: impl Into<String>, exit_code: u8) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandSpec {
    program: &'static str,
    arguments: Vec<&'static str>,
    environment: Vec<(&'static str, &'static str)>,
}

impl CommandSpec {
    pub(crate) fn new<const N: usize>(program: &'static str, arguments: [&'static str; N]) -> Self {
        Self {
            program,
            arguments: arguments.to_vec(),
            environment: Vec::new(),
        }
    }

    pub(crate) fn cargo<const N: usize>(arguments: [&'static str; N]) -> Self {
        Self::cargo_with_environment(arguments, [])
    }

    pub(crate) fn cargo_with_environment<const N: usize, const M: usize>(
        arguments: [&'static str; N],
        environment: [(&'static str, &'static str); M],
    ) -> Self {
        Self {
            program: "cargo",
            arguments: arguments.to_vec(),
            environment: environment.to_vec(),
        }
    }

    pub(crate) fn display(&self) -> String {
        let environment = self
            .environment
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        let command = std::iter::once(self.program)
            .chain(self.arguments.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        if environment.is_empty() {
            command
        } else {
            format!("{environment} {command}")
        }
    }

    pub(crate) fn process(&self, repository: &Path) -> Command {
        let mut command = Command::new(self.program);
        command
            .args(&self.arguments)
            .envs(self.environment.iter().copied())
            .current_dir(repository);
        command
    }
}

pub(crate) fn run_command(repository: &Path, command: &CommandSpec) -> Result<(), Failure> {
    println!("$ {}", command.display());
    let status = command
        .process(repository)
        .status()
        .map_err(|error| Failure::spawn(command, &error))?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::process(command, status.code()))
    }
}
