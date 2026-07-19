//! Operating-system adapter implementations for OpenManic application ports.
//!
//! This crate owns platform capability detection and normalized evidence, but it never persists
//! or renders data. Future callbacks must perform bounded work. Any necessary unsafe code stays
//! inside private adapter modules behind safe interfaces.

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(all(feature = "platform-windows", feature = "platform-linux"))]
compile_error!("select exactly one platform family: platform-windows or platform-linux");
