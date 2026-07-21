//! Windows MVP release gates that are unsuitable for the ordinary edit loop.

use std::{env, fs, path::Path};

use crate::command::{CommandSpec, Failure, run_command};

pub(crate) fn run(repository: &Path) -> Result<(), Failure> {
    if !cfg!(target_os = "windows") {
        return Err(Failure::check(
            "release-check must run on Windows 11 x86-64 for the MVP artifact",
        ));
    }

    for command in feature_and_build_commands() {
        run_command(repository, &command)?;
    }

    report_artifact_size(repository)?;
    run_command(repository, &package_built_artifact_command())?;
    print_manual_smoke_prerequisites();
    Ok(())
}

pub(crate) fn package(repository: &Path) -> Result<(), Failure> {
    if !cfg!(target_os = "windows") {
        return Err(Failure::check("package-windows must run on Windows x86-64"));
    }
    run_command(repository, &package_command())
}

fn package_command() -> CommandSpec {
    CommandSpec::new(
        "powershell",
        [
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "scripts/package-windows.ps1",
        ],
    )
}

fn package_built_artifact_command() -> CommandSpec {
    CommandSpec::new(
        "powershell",
        [
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "scripts/package-windows.ps1",
            "-SkipBuild",
        ],
    )
}

fn feature_and_build_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::cargo([
            "check",
            "-p",
            "openmanic",
            "--no-default-features",
            "--features",
            "renderer-wgpu,platform-windows",
            "--locked",
        ]),
        CommandSpec::cargo([
            "check",
            "-p",
            "openmanic",
            "--no-default-features",
            "--features",
            "renderer-glow,platform-windows",
            "--locked",
        ]),
        CommandSpec::cargo([
            "build",
            "-p",
            "openmanic",
            "--release",
            "--no-default-features",
            "--features",
            "renderer-wgpu,platform-windows",
            "--locked",
        ]),
    ]
}

fn report_artifact_size(repository: &Path) -> Result<(), Failure> {
    let target = env::var_os("CARGO_TARGET_DIR").map_or_else(
        || repository.join("target"),
        |configured| {
            let configured = std::path::PathBuf::from(configured);
            if configured.is_absolute() {
                configured
            } else {
                repository.join(configured)
            }
        },
    );
    let artifact = target.join("release/openmanic.exe");
    let size = fs::metadata(&artifact)
        .map_err(|error| {
            Failure::check(format!(
                "release artifact `{}` is unavailable after the build: {error}",
                artifact.display()
            ))
        })?
        .len();
    println!("release artifact: {} ({size} bytes)", artifact.display());
    Ok(())
}

fn print_manual_smoke_prerequisites() {
    println!("manual Windows 11 smoke evidence is required before release:");
    println!(
        "- run the portable executable from a clean, writable directory without a runtime install"
    );
    println!("- verify single-instance activation, tray recovery, and autostart path quoting");
    println!(
        "- verify lock/unlock, sleep/resume, Explorer restart, and graceful shutdown behavior"
    );
    println!(
        "- record OS build, CPU, GPU/driver, display scale, artifact hash, and observed results"
    );
}

#[cfg(test)]
mod tests {
    use super::{feature_and_build_commands, package_built_artifact_command, package_command};

    #[test]
    fn checks_renderers_separately_and_builds_only_wgpu_release() {
        let commands = feature_and_build_commands()
            .iter()
            .map(crate::command::CommandSpec::display)
            .collect::<Vec<_>>();
        assert_eq!(commands.len(), 3);
        assert!(commands[0].contains("renderer-wgpu,platform-windows"));
        assert!(commands[1].contains("renderer-glow,platform-windows"));
        assert!(commands[2].contains("build -p openmanic --release"));
        assert!(
            commands
                .iter()
                .all(|command| !command.contains("--all-features"))
        );
    }

    #[test]
    fn portable_package_uses_the_reviewed_windows_script() {
        assert_eq!(
            package_command().display(),
            "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows.ps1"
        );
        assert!(
            package_built_artifact_command()
                .display()
                .ends_with("-SkipBuild")
        );
    }
}
