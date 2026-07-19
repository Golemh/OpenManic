//! Deterministic building blocks for OpenManic performance fixtures.
//!
//! This crate owns seedable pseudo-randomness, manually advanced test time, and
//! small input/output test doubles. It deliberately does not own fixture
//! scenarios or production ports; those are introduced after their contracts
//! are frozen. It depends only on the Rust standard library and has no
//! persistence or threading assumptions.

#![forbid(unsafe_code)]

pub mod clock;
pub mod random;
pub mod scenarios;
pub mod scripted;
pub mod serialization;

pub use clock::{ManualClock, MonotonicTicks, UtcMicros};
pub use random::SeededPrng;
pub use scenarios::{Scenario, ScenarioFixture, ScenarioMetadata};
pub use scripted::{RecordingSink, ScriptedInput};
