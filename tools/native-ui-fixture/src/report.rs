//! Versioned JSONL diagnostics for native renderer comparison runs.

use crate::arguments::Arguments;
use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Outcome recorded when the native event loop returns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunOutcome {
    /// The fixture event loop ended without a renderer initialization error.
    Completed,
    /// The selected renderer failed. The fixture never tries the other renderer.
    RendererFailure(String),
}

/// Executable identity observed before the event loop begins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactObservation {
    /// The executable was available for stable measurement.
    Available {
        /// Basename only, avoiding a machine-specific output path in diagnostics.
        file_name: String,
        /// Exact executable byte count.
        size_bytes: u64,
        /// Stable content checksum used to associate a report with its artifact.
        content_hash: String,
    },
    /// Artifact metadata could not be collected and must not be inferred.
    NotCollected {
        /// Safe diagnostic reason.
        reason: String,
    },
}

impl ArtifactObservation {
    /// Observes the running executable without invoking an external program.
    pub(crate) fn observe_current_executable() -> Self {
        match env::current_exe().and_then(observe_artifact) {
            Ok(observation) => observation,
            Err(error) => Self::NotCollected {
                reason: error.to_string(),
            },
        }
    }
}

/// Environment identity known to the fixture without platform-specific probes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentObservation {
    os_family: String,
    os_name: String,
    architecture: String,
    process_id: u32,
    hardware_manifest: ManifestObservation,
}

impl EnvironmentObservation {
    /// Captures process-neutral metadata and references a caller-supplied hardware manifest.
    pub(crate) fn from_arguments(arguments: &Arguments) -> Self {
        Self {
            os_family: env::consts::FAMILY.to_owned(),
            os_name: env::consts::OS.to_owned(),
            architecture: env::consts::ARCH.to_owned(),
            process_id: std::process::id(),
            hardware_manifest: ManifestObservation::from_path(
                arguments.environment_manifest.as_deref(),
            ),
        }
    }
}

/// Caller-supplied named-hardware manifest identity.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestObservation {
    status: &'static str,
    file_name: Option<String>,
    content_hash: Option<String>,
    reason: Option<String>,
}

impl ManifestObservation {
    fn from_path(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self {
                status: "not_provided",
                file_name: None,
                content_hash: None,
                reason: Some(
                    "a named-hardware manifest is required for release evidence".to_owned(),
                ),
            };
        };
        match content_hash(path) {
            Ok(content_hash) => Self {
                status: "referenced",
                file_name: file_name(path),
                content_hash: Some(content_hash),
                reason: None,
            },
            Err(error) => Self {
                status: "unavailable",
                file_name: file_name(path),
                content_hash: None,
                reason: Some(error.to_string()),
            },
        }
    }
}

/// A bounded frame sample collected after the requested warm-up period.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameSample {
    pub(crate) frame_index: u32,
    pub(crate) scripted_interaction: &'static str,
    pub(crate) ui_cpu_ns: u64,
    pub(crate) dense_paint_preparation_ns: u64,
    pub(crate) observed_frame_cadence_ns: Option<u64>,
}

/// State gathered by the eframe app and materialized only after the event loop exits.
#[derive(Clone, Debug)]
pub(crate) struct RunMeasurements {
    schema_version: u8,
    renderer: &'static str,
    build_profile: &'static str,
    scenario: String,
    seed: u64,
    raw_interval_count: usize,
    fixture_checksum: String,
    requested_frame_count: u32,
    warmup_frame_count: u32,
    git_revision: Option<String>,
    lockfile_hash: Option<String>,
    artifact: ArtifactObservation,
    environment: EnvironmentObservation,
    shell_ready_ns: Option<u64>,
    frames: Vec<FrameSample>,
    memory_checkpoints: Vec<MemoryCheckpoint>,
    lock_poisoned: bool,
}

impl RunMeasurements {
    /// Initializes a diagnostic record before the renderer is started.
    #[expect(
        clippy::too_many_arguments,
        reason = "the immutable run identity is recorded together"
    )]
    pub(crate) fn new(
        schema_version: u8,
        renderer: &'static str,
        arguments: &Arguments,
        scenario: &str,
        raw_interval_count: usize,
        fixture_checksum: String,
        artifact: ArtifactObservation,
        environment: EnvironmentObservation,
    ) -> Self {
        Self {
            schema_version,
            renderer,
            build_profile: if cfg!(debug_assertions) {
                "debug-like"
            } else {
                "release-like"
            },
            scenario: scenario.to_owned(),
            seed: arguments.seed,
            raw_interval_count,
            fixture_checksum,
            requested_frame_count: arguments.frame_count,
            warmup_frame_count: arguments.warmup_frame_count,
            git_revision: arguments.git_revision.clone(),
            lockfile_hash: arguments.lockfile_hash.clone(),
            artifact,
            environment,
            shell_ready_ns: None,
            frames: Vec::with_capacity(
                (arguments.frame_count - arguments.warmup_frame_count) as usize,
            ),
            memory_checkpoints: Vec::with_capacity(3),
            lock_poisoned: false,
        }
    }

    /// Records the first native fixture frame as a diagnostic shell-ready observation.
    pub(crate) fn record_shell_ready(&mut self, elapsed_ns: u64) {
        self.shell_ready_ns.get_or_insert(elapsed_ns);
    }

    /// Adds a post-warm-up frame sample.
    pub(crate) fn record_frame(&mut self, frame: FrameSample) {
        self.frames.push(frame);
    }

    /// Records a memory observation hook without pretending that memory was sampled in-process.
    pub(crate) fn record_memory_checkpoint(&mut self, checkpoint: &'static str, frame_index: u32) {
        self.memory_checkpoints.push(MemoryCheckpoint {
            checkpoint,
            frame_index,
            status: "not_collected",
            reason: "sample Windows working set externally at this deterministic checkpoint; no process-inspection dependency is linked",
        });
    }

    /// Marks a recovered measurement lock poison condition for later disclosure.
    pub(crate) fn record_lock_poisoning(&mut self) {
        self.lock_poisoned = true;
    }

    /// Produces a stable report snapshot after eframe returns.
    pub(crate) fn finish(&self, outcome: RunOutcome) -> MeasurementReport {
        MeasurementReport {
            schema_version: self.schema_version,
            renderer: self.renderer,
            build_profile: self.build_profile,
            scenario: self.scenario.clone(),
            seed: self.seed,
            raw_interval_count: self.raw_interval_count,
            fixture_checksum: self.fixture_checksum.clone(),
            requested_frame_count: self.requested_frame_count,
            warmup_frame_count: self.warmup_frame_count,
            git_revision: self.git_revision.clone(),
            lockfile_hash: self.lockfile_hash.clone(),
            artifact: self.artifact.clone(),
            environment: self.environment.clone(),
            shell_ready_ns: self.shell_ready_ns,
            frames: self.frames.clone(),
            memory_checkpoints: self.memory_checkpoints.clone(),
            lock_poisoned: self.lock_poisoned,
            outcome,
        }
    }
}

/// Applies a short, explicit update to app-owned diagnostic measurements.
pub(crate) fn update_measurements(
    measurements: &Arc<Mutex<RunMeasurements>>,
    update: impl FnOnce(&mut RunMeasurements),
) {
    match measurements.lock() {
        Ok(mut measurements) => update(&mut measurements),
        Err(poisoned) => {
            let mut measurements = poisoned.into_inner();
            measurements.record_lock_poisoning();
            update(&mut measurements);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MemoryCheckpoint {
    checkpoint: &'static str,
    frame_index: u32,
    status: &'static str,
    reason: &'static str,
}

/// Complete output that is encoded as JSONL after the app exits.
#[derive(Clone, Debug)]
pub(crate) struct MeasurementReport {
    schema_version: u8,
    renderer: &'static str,
    build_profile: &'static str,
    scenario: String,
    seed: u64,
    raw_interval_count: usize,
    fixture_checksum: String,
    requested_frame_count: u32,
    warmup_frame_count: u32,
    git_revision: Option<String>,
    lockfile_hash: Option<String>,
    artifact: ArtifactObservation,
    environment: EnvironmentObservation,
    shell_ready_ns: Option<u64>,
    frames: Vec<FrameSample>,
    memory_checkpoints: Vec<MemoryCheckpoint>,
    lock_poisoned: bool,
    outcome: RunOutcome,
}

/// Writes a new, never-overwritten JSONL report.
///
/// # Errors
///
/// Returns I/O errors creating or flushing the caller-selected output file.
pub(crate) fn write_report(path: &Path, report: &MeasurementReport) -> io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    for line in report.json_lines() {
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

impl MeasurementReport {
    fn json_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.frames.len() + self.memory_checkpoints.len() + 7);
        lines.push(self.run_line());
        lines.push(self.environment_line());
        lines.push(self.artifact_line());
        lines.push(self.fixture_line());
        if let Some(shell_ready_ns) = self.shell_ready_ns {
            lines.push(format!(
                "{{\"schema_version\":{},\"record\":\"shell_ready\",\"fixture_shell_ready_ns\":{shell_ready_ns},\"definition\":\"elapsed from fixture main entry to its first eframe update; not an OpenManic product-shell measurement\"}}",
                self.schema_version
            ));
        }
        lines.extend(self.frames.iter().map(|frame| self.frame_line(frame)));
        lines.extend(
            self.memory_checkpoints
                .iter()
                .map(|checkpoint| self.memory_line(checkpoint)),
        );
        lines.push(self.summary_line());
        lines.push(self.outcome_line());
        lines
    }

    fn run_line(&self) -> String {
        format!(
            "{{\"schema_version\":{},\"record\":\"run\",\"diagnostic_only\":true,\"renderer\":{},\"build_profile\":{},\"requested_frame_count\":{},\"warmup_frame_count\":{},\"git_revision\":{},\"lockfile_hash\":{},\"lock_poisoned\":{}}}",
            self.schema_version,
            json_string(self.renderer),
            json_string(self.build_profile),
            self.requested_frame_count,
            self.warmup_frame_count,
            optional_json_string(self.git_revision.as_deref()),
            optional_json_string(self.lockfile_hash.as_deref()),
            self.lock_poisoned,
        )
    }

    fn environment_line(&self) -> String {
        let manifest = &self.environment.hardware_manifest;
        format!(
            "{{\"schema_version\":{},\"record\":\"environment\",\"os_family\":{},\"os_name\":{},\"architecture\":{},\"process_id\":{},\"windows_build\":\"not_collected\",\"cpu\":\"not_collected\",\"ram\":\"not_collected\",\"gpu\":\"not_collected\",\"gpu_driver\":\"not_collected\",\"display_refresh_hz\":\"not_collected\",\"display_scaling\":\"not_collected\",\"power_mode\":\"not_collected\",\"storage\":\"not_collected\",\"antivirus_configuration\":\"not_collected\",\"hardware_manifest_status\":{},\"hardware_manifest_file_name\":{},\"hardware_manifest_content_hash\":{},\"hardware_manifest_reason\":{}}}",
            self.schema_version,
            json_string(&self.environment.os_family),
            json_string(&self.environment.os_name),
            json_string(&self.environment.architecture),
            self.environment.process_id,
            json_string(manifest.status),
            optional_json_string(manifest.file_name.as_deref()),
            optional_json_string(manifest.content_hash.as_deref()),
            optional_json_string(manifest.reason.as_deref()),
        )
    }

    fn artifact_line(&self) -> String {
        match &self.artifact {
            ArtifactObservation::Available {
                file_name,
                size_bytes,
                content_hash,
            } => format!(
                "{{\"schema_version\":{},\"record\":\"artifact\",\"status\":\"collected\",\"file_name\":{},\"size_bytes\":{size_bytes},\"content_hash\":{}}}",
                self.schema_version,
                json_string(file_name),
                json_string(content_hash),
            ),
            ArtifactObservation::NotCollected { reason } => format!(
                "{{\"schema_version\":{},\"record\":\"artifact\",\"status\":\"not_collected\",\"reason\":{}}}",
                self.schema_version,
                json_string(reason),
            ),
        }
    }

    fn fixture_line(&self) -> String {
        format!(
            "{{\"schema_version\":{},\"record\":\"fixture\",\"scenario\":{},\"seed\":{},\"raw_interval_count\":{},\"fixture_checksum\":{}}}",
            self.schema_version,
            json_string(&self.scenario),
            self.seed,
            self.raw_interval_count,
            json_string(&self.fixture_checksum),
        )
    }

    fn frame_line(&self, frame: &FrameSample) -> String {
        format!(
            "{{\"schema_version\":{},\"record\":\"frame\",\"frame_index\":{},\"scripted_interaction\":{},\"ui_cpu_ns\":{},\"dense_paint_preparation_ns\":{},\"observed_frame_cadence_ns\":{},\"observed_frame_cadence_definition\":\"start-to-start eframe update cadence; includes event-loop, selected-renderer submission, presentation pacing, and scheduling, and is not a direct GPU completion measurement\",\"gpu_submission_duration\":\"not_observed_by_eframe_app_callback\"}}",
            self.schema_version,
            frame.frame_index,
            json_string(frame.scripted_interaction),
            frame.ui_cpu_ns,
            frame.dense_paint_preparation_ns,
            optional_u64(frame.observed_frame_cadence_ns),
        )
    }

    fn memory_line(&self, checkpoint: &MemoryCheckpoint) -> String {
        format!(
            "{{\"schema_version\":{},\"record\":\"memory_checkpoint\",\"checkpoint\":{},\"frame_index\":{},\"status\":{},\"reason\":{}}}",
            self.schema_version,
            json_string(checkpoint.checkpoint),
            checkpoint.frame_index,
            json_string(checkpoint.status),
            json_string(checkpoint.reason),
        )
    }

    fn summary_line(&self) -> String {
        let ui_cpu = self
            .frames
            .iter()
            .map(|frame| frame.ui_cpu_ns)
            .collect::<Vec<_>>();
        let dense_paint = self
            .frames
            .iter()
            .map(|frame| frame.dense_paint_preparation_ns)
            .collect::<Vec<_>>();
        let cadence = self
            .frames
            .iter()
            .filter_map(|frame| frame.observed_frame_cadence_ns)
            .collect::<Vec<_>>();
        format!(
            "{{\"schema_version\":{},\"record\":\"summary\",\"sample_count\":{},\"percentile_method\":\"nearest-rank: sorted index ceil(p*n)-1\",\"ui_cpu_p50_ns\":{},\"ui_cpu_p95_ns\":{},\"dense_paint_preparation_p50_ns\":{},\"dense_paint_preparation_p95_ns\":{},\"observed_frame_cadence_p50_ns\":{},\"observed_frame_cadence_p95_ns\":{}}}",
            self.schema_version,
            self.frames.len(),
            optional_u64(nearest_rank(&ui_cpu, 50)),
            optional_u64(nearest_rank(&ui_cpu, 95)),
            optional_u64(nearest_rank(&dense_paint, 50)),
            optional_u64(nearest_rank(&dense_paint, 95)),
            optional_u64(nearest_rank(&cadence, 50)),
            optional_u64(nearest_rank(&cadence, 95)),
        )
    }

    fn outcome_line(&self) -> String {
        let (outcome, detail) = match &self.outcome {
            RunOutcome::Completed => ("completed", None),
            RunOutcome::RendererFailure(detail) => ("renderer_failure", Some(detail.as_str())),
        };
        format!(
            "{{\"schema_version\":{},\"record\":\"outcome\",\"outcome\":{},\"detail\":{},\"fallback_attempted\":false,\"release_evidence\":false}}",
            self.schema_version,
            json_string(outcome),
            optional_json_string(detail),
        )
    }
}

fn observe_artifact(path: PathBuf) -> io::Result<ArtifactObservation> {
    let metadata = fs::metadata(&path)?;
    Ok(ArtifactObservation::Available {
        file_name: file_name(&path).unwrap_or_else(|| "unknown-artifact".to_owned()),
        size_bytes: metadata.len(),
        content_hash: content_hash(&path)?,
    })
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|file_name| file_name.to_string_lossy().into_owned())
}

fn content_hash(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn nearest_rank(values: &[u64], percentile: u8) -> Option<u64> {
    if values.is_empty() || percentile == 0 || percentile > 100 {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * usize::from(percentile)).div_ceil(100);
    sorted.get(rank.saturating_sub(1)).copied()
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn optional_json_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                let code = character as u32;
                escaped.push_str(&format!("\\u{code:04x}"));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{json_escape, nearest_rank};

    #[test]
    fn nearest_rank_uses_the_documented_index() {
        let values = [90, 10, 40, 20, 80, 30, 70, 50, 60, 100];
        assert_eq!(nearest_rank(&values, 50), Some(50));
        assert_eq!(nearest_rank(&values, 95), Some(100));
    }

    #[test]
    fn nearest_rank_rejects_empty_and_invalid_percentiles() {
        assert_eq!(nearest_rank(&[], 50), None);
        assert_eq!(nearest_rank(&[1, 2], 0), None);
        assert_eq!(nearest_rank(&[1, 2], 101), None);
    }

    #[test]
    fn json_escape_keeps_jsonl_records_single_line() {
        assert_eq!(
            json_escape("quote\" slash\\\nline"),
            "quote\\\" slash\\\\\\nline"
        );
    }
}
