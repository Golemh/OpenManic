//! Isolated native eframe measurement fixture for renderer comparison.
//!
//! This executable owns only deterministic synthetic rendering and diagnostic
//! measurement records. It deliberately does not define or consume OpenManic
//! product-layer contracts, storage, platform adapters, or production UI code.

#![forbid(unsafe_code)]

#[cfg(all(feature = "renderer-wgpu", feature = "renderer-glow"))]
compile_error!("select exactly one renderer: renderer-wgpu or renderer-glow");

#[cfg(not(any(feature = "renderer-wgpu", feature = "renderer-glow")))]
compile_error!("select one renderer: renderer-wgpu or renderer-glow");

mod arguments;
mod report;
mod workload;

use crate::{
    arguments::Arguments,
    report::{
        ArtifactObservation, EnvironmentObservation, RunMeasurements, RunOutcome, write_report,
    },
    workload::NativeFixture,
};
use fixture_generator::serialization::write_jsonl;
use std::{
    io,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::Instant,
};

const FIXTURE_SCHEMA_VERSION: u8 = 1;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("native-ui-fixture: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let arguments = Arguments::parse(std::env::args_os().skip(1))?;
    let fixture = arguments.scenario.generate(arguments.seed);
    let fixture_checksum = fixture_checksum(&fixture).map_err(|error| error.to_string())?;
    let artifact = ArtifactObservation::observe_current_executable();
    let environment = EnvironmentObservation::from_arguments(&arguments);
    let measurements = Arc::new(Mutex::new(RunMeasurements::new(
        FIXTURE_SCHEMA_VERSION,
        Arguments::renderer_name(),
        &arguments,
        fixture.scenario.name(),
        fixture.metadata.raw_interval_count,
        fixture_checksum,
        artifact,
        environment,
    )));

    let launch_started = Instant::now();
    let app_measurements = Arc::clone(&measurements);
    let app_arguments = arguments.clone();
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    let eframe_result = eframe::run_native(
        "OpenManic native UI fixture (diagnostic)",
        native_options,
        Box::new(move |_creation_context| {
            Ok(Box::new(NativeFixture::new(
                fixture,
                app_arguments,
                launch_started,
                app_measurements,
            )))
        }),
    );

    let outcome = match eframe_result {
        Ok(()) => RunOutcome::Completed,
        Err(error) => RunOutcome::RendererFailure(error.to_string()),
    };
    let report = match measurements.lock() {
        Ok(measurements) => measurements.finish(outcome),
        Err(poisoned) => {
            let mut measurements = poisoned.into_inner();
            measurements.record_lock_poisoning();
            measurements.finish(outcome)
        }
    };
    write_report(&arguments.output, &report).map_err(|error| error.to_string())
}

fn fixture_checksum(fixture: &fixture_generator::ScenarioFixture) -> io::Result<String> {
    let mut sink = io::sink();
    let checksum = write_jsonl(&mut sink, fixture)?;
    Ok(format!("fnv1a64:{checksum:016x}"))
}

#[cfg(test)]
mod tests {
    use super::fixture_checksum;
    use fixture_generator::Scenario;
    use std::io;

    #[test]
    fn checksum_repeats_for_the_frozen_dense_fixture() -> io::Result<()> {
        let fixture = Scenario::Dense10000IntervalRange.generate(2_026_030);
        let first = fixture_checksum(&fixture)?;
        let second = fixture_checksum(&fixture)?;
        assert_eq!(first, second);
        assert_eq!(first, "fnv1a64:5b78a7e59da804a8");
        Ok(())
    }
}
