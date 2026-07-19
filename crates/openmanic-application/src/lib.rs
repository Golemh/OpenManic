//! OpenManic use cases, ports, commands, events, services, runtime supervision, and snapshots.
//!
//! This crate may depend on domain policy, but it deliberately does not depend on concrete GUI,
//! storage, or platform adapters. Future concurrency is owned here and must use explicit bounded
//! communication and shutdown protocols.

#![forbid(unsafe_code)]
