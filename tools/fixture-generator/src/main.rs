//! Command-line entry point for deterministic fixture generation.

#![forbid(unsafe_code)]

use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    seed: u64,
    output: PathBuf,
    scenario: String,
}

fn main() -> ExitCode {
    match parse_arguments(env::args_os().skip(1)) {
        Ok(arguments) => {
            println!(
                "scenario `{}` is reserved for milestone 2; seed={} output={}",
                arguments.scenario,
                arguments.seed,
                arguments.output.display()
            );
            ExitCode::from(2)
        }
        Err(message) => {
            eprintln!("fixture-generator: {message}");
            ExitCode::from(64)
        }
    }
}

fn parse_arguments(arguments: impl IntoIterator<Item = OsString>) -> Result<Arguments, String> {
    let mut values = arguments.into_iter();
    let mut seed = None;
    let mut output = None;
    let mut scenario = None;
    while let Some(flag) = values.next() {
        let value = values.next().ok_or_else(|| {
            format!(
                "missing value for `{}`; expected --seed, --output, or --scenario",
                flag.to_string_lossy()
            )
        })?;
        match flag.to_str() {
            Some("--seed") => seed = Some(parse_seed(value)?),
            Some("--output") => output = Some(PathBuf::from(value)),
            Some("--scenario") => scenario = Some(required_unicode(value, "scenario")?),
            _ => return Err(format!("unknown argument `{}`", flag.to_string_lossy())),
        }
    }
    Ok(Arguments {
        seed: seed.ok_or_else(|| "missing required --seed value".to_owned())?,
        output: output.ok_or_else(|| "missing required --output value".to_owned())?,
        scenario: scenario.ok_or_else(|| "missing required --scenario value".to_owned())?,
    })
}

fn parse_seed(value: OsString) -> Result<u64, String> {
    let text = required_unicode(value, "seed")?;
    text.parse::<u64>()
        .map_err(|_| format!("invalid seed `{text}`; expected an unsigned 64-bit integer"))
}

fn required_unicode(value: OsString, name: &str) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| format!("{name} must be valid Unicode"))
}

#[cfg(test)]
mod tests {
    use super::{Arguments, parse_arguments};
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn parses_seed_output_and_scenario() {
        let arguments = parse_arguments(
            [
                "--seed",
                "42",
                "--output",
                "fixtures/out",
                "--scenario",
                "normal-workday",
            ]
            .map(OsString::from),
        );
        assert_eq!(
            arguments,
            Ok(Arguments {
                seed: 42,
                output: PathBuf::from("fixtures/out"),
                scenario: "normal-workday".to_owned()
            })
        );
    }

    #[test]
    fn rejects_missing_and_invalid_values() {
        assert!(parse_arguments(["--seed", "not-a-number"].map(OsString::from)).is_err());
        assert!(parse_arguments(["--seed"].map(OsString::from)).is_err());
        assert!(parse_arguments(["--unknown", "value"].map(OsString::from)).is_err());
    }
}
