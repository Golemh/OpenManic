//! OpenManic process composition and lifecycle boundary.
//!
//! This crate wires the GUI, application services, storage, and platform adapters. It deliberately
//! owns no product policy. Startup will establish diagnostics and data ownership before workers,
//! while shutdown will coordinate checkpoints and joins explicitly.

#![forbid(unsafe_code)]

/// Process bootstrap sequencing and first-launch gates.
pub mod bootstrap;
/// Minimal command-line parsing for process bootstrap.
pub mod cli;
/// Primary-owned vertical-slice composition of accepted subsystem boundaries.
pub mod composition;
/// Data-root discovery, validation, locator persistence, and writer locking.
pub mod data_root;
/// Privacy-safe local bootstrap diagnostics and panic markers.
pub mod diagnostics;
