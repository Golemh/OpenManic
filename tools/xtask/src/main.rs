//! Repository quality and release automation.

mod command;
mod dependency;
mod docs;
mod release;

use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

use command::{CommandSpec, Failure, run_command};

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {}", error.message());
            ExitCode::from(error.exit_code())
        }
    }
}

fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), Failure> {
    let repository = repository_root()?;
    match Task::parse(arguments)? {
        Task::Help => {
            print_help();
            Ok(())
        }
        Task::Quality => run_quality(&repository),
        Task::DocsCheck => run_docs_check(&repository),
        Task::DependencyCheck => dependency::run(&repository),
        Task::ReleaseCheck => run_release_check(&repository),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Task {
    Help,
    Quality,
    DocsCheck,
    DependencyCheck,
    ReleaseCheck,
}

impl Task {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, Failure> {
        let mut arguments = arguments.into_iter();
        let Some(command) = arguments.next() else {
            return Ok(Self::Help);
        };
        if arguments.next().is_some() {
            return Err(Failure::usage(
                "expected exactly one xtask command; run `cargo xtask help`",
            ));
        }

        match command.to_str() {
            Some("help" | "--help" | "-h") => Ok(Self::Help),
            Some("quality") => Ok(Self::Quality),
            Some("docs-check") => Ok(Self::DocsCheck),
            Some("dependency-check") => Ok(Self::DependencyCheck),
            Some("release-check") => Ok(Self::ReleaseCheck),
            Some(other) => Err(Failure::usage(format!(
                "unknown command `{other}`; run `cargo xtask help`"
            ))),
            None => Err(Failure::usage(
                "command must be valid Unicode; run `cargo xtask help`",
            )),
        }
    }
}

fn print_help() {
    println!(
        "Usage: cargo xtask <command>\n\nCommands:\n  quality           Run format, check, lint, test, rustdoc, and docs checks\n  docs-check        Validate documentation structure and local links\n  dependency-check  Run the pinned cargo-deny policy checks\n  release-check     Run quality, dependency, feature, build, and release gates\n  help              Show this help"
    );
}

fn repository_root() -> Result<PathBuf, Failure> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|tools| tools.parent())
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| Failure::internal("could not determine the repository root"))
}

fn run_quality(repository: &std::path::Path) -> Result<(), Failure> {
    for command in quality_commands() {
        run_command(repository, &command)?;
    }

    println!("$ cargo xtask docs-check");
    run_docs_check(repository)
}

fn run_docs_check(repository: &std::path::Path) -> Result<(), Failure> {
    docs::check(&repository.join("docs/gui")).map_err(Failure::check)
}

fn run_release_check(repository: &std::path::Path) -> Result<(), Failure> {
    run_quality(repository)?;
    dependency::run(repository)?;
    release::run(repository)
}

fn quality_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::cargo(["fmt", "--all", "--", "--check"]),
        CommandSpec::cargo(["check", "--workspace", "--all-targets", "--locked"]),
        CommandSpec::cargo([
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        CommandSpec::cargo(["test", "--workspace", "--locked"]),
        CommandSpec::cargo_with_environment(
            ["doc", "--workspace", "--no-deps", "--locked"],
            [("RUSTDOCFLAGS", "-D warnings")],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{CommandSpec, Task, quality_commands};
    use std::ffi::OsString;

    #[test]
    fn parses_supported_commands_and_help() {
        for (input, expected) in [
            (vec![], Task::Help),
            (vec!["help"], Task::Help),
            (vec!["quality"], Task::Quality),
            (vec!["docs-check"], Task::DocsCheck),
            (vec!["dependency-check"], Task::DependencyCheck),
            (vec!["release-check"], Task::ReleaseCheck),
        ] {
            let arguments = input.into_iter().map(OsString::from);
            assert_eq!(
                Task::parse(arguments).map_err(|error| error.to_string()),
                Ok(expected)
            );
        }
    }

    #[test]
    fn rejects_unknown_or_extra_commands() {
        assert!(Task::parse([OsString::from("unknown")]).is_err());
        assert!(Task::parse([OsString::from("quality"), OsString::from("extra")]).is_err());
    }

    #[test]
    fn constructs_the_documented_quality_chain() {
        let rendered = quality_commands()
            .iter()
            .map(CommandSpec::display)
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            vec![
                "cargo fmt --all -- --check",
                "cargo check --workspace --all-targets --locked",
                "cargo clippy --workspace --all-targets --locked -- -D warnings",
                "cargo test --workspace --locked",
                "RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked",
            ]
        );
    }
}
