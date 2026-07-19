//! Native low-fidelity UI direction spike for the OpenManic MVP.
//!
//! This executable is a review tool, not production UI architecture. It owns only deterministic
//! mock snapshots, local interaction reduction, and visual alternatives. It deliberately has no
//! storage, platform, runtime, or application-contract dependency.

#![forbid(unsafe_code)]

#[cfg(all(feature = "renderer-wgpu", feature = "renderer-glow"))]
compile_error!("select exactly one renderer for the UI direction spike");

#[cfg(not(any(feature = "renderer-wgpu", feature = "renderer-glow")))]
compile_error!("select a renderer for the UI direction spike");

mod model;
mod render;

use eframe::egui;
use render::UiDirectionApp;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1_280.0, 860.0])
        .with_min_inner_size([720.0, 600.0]);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "OpenManic UI direction spike",
        options,
        Box::new(|_creation_context| Ok(Box::<UiDirectionApp>::default())),
    )
}
