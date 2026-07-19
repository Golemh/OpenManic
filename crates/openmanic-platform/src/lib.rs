//! Operating-system adapter implementations for OpenManic application ports.
//!
//! This crate owns platform capability detection and normalized evidence, but it never persists
//! or renders data. Future callbacks must perform bounded work. Any necessary unsafe code stays
//! inside private adapter modules behind safe interfaces.

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(all(feature = "platform-windows", feature = "platform-linux"))]
compile_error!("select exactly one platform family: platform-windows or platform-linux");

mod adapter;
mod capabilities;
mod fake;
mod windows_control;
mod windows_raw;

pub use adapter::{
    AdapterObservation, AdapterObservationKind, AdapterPublishResult, AdapterPublishStatus,
    EvidencePublishResult, ObservationIgnoredReason, PlatformActivityAdapter,
    PlatformEventNormalizer, TrackingEvidenceSink,
};
pub use capabilities::{
    AdapterAvailability, AvailabilityTransition, Capability, CapabilitySet, DeliveryCapability,
    FieldSupport, FocusScope, HelperRequirement, PermissionModel, PlatformCapabilities,
};
pub use fake::{FakeEvidenceSink, FakeEvidenceSinkError, FakePlatformAdapter};
pub use windows_control::{
    WINDOWS_FOREGROUND_INGRESS_CAPACITY, WindowsControlAdapter, WindowsControlDrain,
};
#[cfg(windows)]
pub use windows_control::{WindowsControlError, WindowsControlWindow};
