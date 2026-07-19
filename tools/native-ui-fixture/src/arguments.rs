//! Command-line parsing for repeatable native fixture invocations.

use fixture_generator::Scenario;
use std::{ffi::OsString, path::PathBuf};

const DEFAULT_SEED: u64 = 2_026_030;
const DEFAULT_FRAME_COUNT: u32 = 360;
const DEFAULT_WARMUP_FRAME_COUNT: u32 = 60;
const MAX_FRAME_COUNT: u32 = 10_000;

/// Validated native-fixture invocation parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Arguments {
    pub(crate) output: PathBuf,
    pub(crate) scenario: Scenario,
    pub(crate) seed: u64,
    pub(crate) frame_count: u32,
    pub(crate) warmup_frame_count: u32,
    pub(crate) git_revision: Option<String>,
    pub(crate) lockfile_hash: Option<String>,
    pub(crate) environment_manifest: Option<PathBuf>,
}

impl Arguments {
    /// Parses an invocation without accessing the filesystem or starting eframe.
    pub(crate) fn parse(values: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut values = values.into_iter();
        let mut output = None;
        let mut scenario = None;
        let mut seed = None;
        let mut frame_count = None;
        let mut warmup_frame_count = None;
        let mut git_revision = None;
        let mut lockfile_hash = None;
        let mut environment_manifest = None;

        while let Some(flag) = values.next() {
            let value = values.next().ok_or_else(|| missing_value(&flag))?;
            match flag.to_str() {
                Some("--output") if output.is_none() => output = Some(PathBuf::from(value)),
                Some("--scenario") if scenario.is_none() => scenario = Some(parse_scenario(value)?),
                Some("--seed") if seed.is_none() => seed = Some(parse_u64(value, "seed")?),
                Some("--frames") if frame_count.is_none() => {
                    frame_count = Some(parse_u32(value, "frames")?);
                }
                Some("--warmup-frames") if warmup_frame_count.is_none() => {
                    warmup_frame_count = Some(parse_u32(value, "warmup frame count")?);
                }
                Some("--git-revision") if git_revision.is_none() => {
                    git_revision = Some(required_unicode(value, "git revision")?);
                }
                Some("--lockfile-hash") if lockfile_hash.is_none() => {
                    lockfile_hash = Some(required_unicode(value, "lockfile hash")?);
                }
                Some("--environment-manifest") if environment_manifest.is_none() => {
                    environment_manifest = Some(PathBuf::from(value));
                }
                Some(
                    "--output"
                    | "--scenario"
                    | "--seed"
                    | "--frames"
                    | "--warmup-frames"
                    | "--git-revision"
                    | "--lockfile-hash"
                    | "--environment-manifest",
                ) => return Err(format!("duplicate flag `{}`", flag.to_string_lossy())),
                _ => return Err(format!("unknown argument `{}`", flag.to_string_lossy())),
            }
        }

        let frame_count = frame_count.unwrap_or(DEFAULT_FRAME_COUNT);
        let warmup_frame_count = warmup_frame_count.unwrap_or(DEFAULT_WARMUP_FRAME_COUNT);
        if !(2..=MAX_FRAME_COUNT).contains(&frame_count) {
            return Err(format!(
                "frames must be between 2 and {MAX_FRAME_COUNT}, received {frame_count}"
            ));
        }
        if warmup_frame_count >= frame_count {
            return Err(format!(
                "warmup frames ({warmup_frame_count}) must be less than frames ({frame_count})"
            ));
        }

        Ok(Self {
            output: output.ok_or_else(|| "missing required --output value".to_owned())?,
            scenario: scenario.unwrap_or(Scenario::Dense10000IntervalRange),
            seed: seed.unwrap_or(DEFAULT_SEED),
            frame_count,
            warmup_frame_count,
            git_revision,
            lockfile_hash,
            environment_manifest,
        })
    }

    /// Returns the renderer compiled into this artifact.
    pub(crate) const fn renderer_name() -> &'static str {
        #[cfg(feature = "renderer-wgpu")]
        {
            "wgpu"
        }
        #[cfg(feature = "renderer-glow")]
        {
            "glow"
        }
    }
}

fn missing_value(flag: &OsString) -> String {
    format!("missing value for `{}`", flag.to_string_lossy())
}

fn parse_scenario(value: OsString) -> Result<Scenario, String> {
    let name = required_unicode(value, "scenario")?;
    Scenario::from_name(&name)
        .ok_or_else(|| format!("unknown frozen scenario `{name}`; pass one OM-030 scenario name"))
}

fn parse_u64(value: OsString, name: &str) -> Result<u64, String> {
    let text = required_unicode(value, name)?;
    text.parse::<u64>()
        .map_err(|_| format!("invalid {name} `{text}`; expected an unsigned integer"))
}

fn parse_u32(value: OsString, name: &str) -> Result<u32, String> {
    let text = required_unicode(value, name)?;
    text.parse::<u32>()
        .map_err(|_| format!("invalid {name} `{text}`; expected an unsigned integer"))
}

fn required_unicode(value: OsString, name: &str) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| format!("{name} must be valid Unicode"))
}

#[cfg(test)]
mod tests {
    use super::Arguments;
    use fixture_generator::Scenario;
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn uses_dense_om030_fixture_and_reproducible_defaults() {
        let parsed = Arguments::parse(["--output", "result.jsonl"].map(OsString::from));
        assert_eq!(
            parsed,
            Ok(Arguments {
                output: PathBuf::from("result.jsonl"),
                scenario: Scenario::Dense10000IntervalRange,
                seed: 2_026_030,
                frame_count: 360,
                warmup_frame_count: 60,
                git_revision: None,
                lockfile_hash: None,
                environment_manifest: None,
            })
        );
    }

    #[test]
    fn rejects_duplicate_and_invalid_measurement_arguments() {
        for values in [
            vec!["--output", "a", "--output", "b"],
            vec!["--output", "a", "--scenario", "missing"],
            vec!["--output", "a", "--frames", "1"],
            vec!["--output", "a", "--frames", "4", "--warmup-frames", "4"],
        ] {
            assert!(Arguments::parse(values.into_iter().map(OsString::from)).is_err());
        }
    }
}
