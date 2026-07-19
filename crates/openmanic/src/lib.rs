//! OpenManic process composition and lifecycle boundary.
//!
//! This crate wires the GUI, application services, storage, and platform adapters. It deliberately
//! owns no product policy. Startup will establish diagnostics and data ownership before workers,
//! while shutdown will coordinate checkpoints and joins explicitly.

#![forbid(unsafe_code)]
