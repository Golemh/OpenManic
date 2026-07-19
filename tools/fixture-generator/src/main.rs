//! Command-line entry point for deterministic fixture generation.

#![forbid(unsafe_code)]

use fixture_generator::{Scenario, serialization::materialize};
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

#[derive(Debug, Eq, PartialEq)]
struct Arguments {
    seed: u64,
    output: PathBuf,
    scenarios: Vec<Scenario>,
}

fn main() -> ExitCode {
    match parse_arguments(env::args_os().skip(1)) {
        Ok(arguments) => match materialize(&arguments.output, arguments.seed, &arguments.scenarios)
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fixture-generator: {error}");
                ExitCode::from(1)
            }
        },
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
        let value = values.next().ok_or_else(|| missing_value(&flag))?;
        match flag.to_str() {
            Some("--seed") if seed.is_none() => seed = Some(parse_seed(value)?),
            Some("--output") if output.is_none() => output = Some(PathBuf::from(value)),
            Some("--scenario") if scenario.is_none() => scenario = Some(parse_scenarios(value)?),
            Some("--seed" | "--output" | "--scenario") => {
                return Err(format!("duplicate flag `{}`", flag.to_string_lossy()));
            }
            _ => return Err(format!("unknown argument `{}`", flag.to_string_lossy())),
        }
    }
    Ok(Arguments {
        seed: seed.ok_or_else(|| "missing required --seed value".to_owned())?,
        output: output.ok_or_else(|| "missing required --output value".to_owned())?,
        scenarios: scenario.ok_or_else(|| "missing required --scenario value".to_owned())?,
    })
}

fn missing_value(flag: &OsString) -> String {
    format!(
        "missing value for `{}`; expected --seed, --output, or --scenario",
        flag.to_string_lossy()
    )
}

fn parse_scenarios(value: OsString) -> Result<Vec<Scenario>, String> {
    let name = required_unicode(value, "scenario")?;
    if name == "all" {
        return Ok(Scenario::all().into());
    }
    Scenario::from_name(&name)
        .map(|scenario| vec![scenario])
        .ok_or_else(|| {
            format!("unknown frozen scenario `{name}`; expected `all` or a configured scenario")
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
    use fixture_generator::Scenario;
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn parses_frozen_name_and_all() {
        let selected = parse_arguments(
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
            selected,
            Ok(Arguments {
                seed: 42,
                output: PathBuf::from("fixtures/out"),
                scenarios: vec![Scenario::NormalWorkday]
            })
        );
        let all = parse_arguments(
            [
                "--seed",
                "42",
                "--output",
                "fixtures/out",
                "--scenario",
                "all",
            ]
            .map(OsString::from),
        )
        .expect("all parses");
        assert_eq!(all.scenarios, Scenario::all());
    }

    #[test]
    fn rejects_duplicates_and_invalid_arguments() {
        for arguments in [
            vec!["--seed", "not-a-number"],
            vec!["--seed"],
            vec!["--unknown", "value"],
            vec![
                "--seed",
                "1",
                "--seed",
                "2",
                "--output",
                "out",
                "--scenario",
                "all",
            ],
            vec!["--seed", "1", "--output", "out", "--scenario", "unknown"],
        ] {
            assert!(parse_arguments(arguments.into_iter().map(OsString::from)).is_err());
        }
    }
}
