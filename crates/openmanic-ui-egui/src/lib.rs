//! egui/eframe presentation for immutable OpenManic application snapshots.
//!
//! This crate owns GUI lifecycle and rendering. It emits typed actions and deliberately cannot
//! depend on concrete storage or platform adapters. Frame updates must not block on I/O, platform
//! calls, recurrence expansion, or full-history aggregation.

#![forbid(unsafe_code)]

#[cfg(all(feature = "renderer-wgpu", feature = "renderer-glow"))]
compile_error!("select exactly one renderer: renderer-wgpu or renderer-glow");

#[cfg(not(any(feature = "renderer-wgpu", feature = "renderer-glow")))]
compile_error!("select one renderer: renderer-wgpu or renderer-glow");
