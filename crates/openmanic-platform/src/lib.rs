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
#[cfg(windows)]
mod windows_autostart;
mod windows_control;
mod windows_identity;
mod windows_lifecycle;
#[cfg(windows)]
mod windows_metadata;
mod windows_raw;
mod windows_single_instance;
mod windows_tray;

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
#[cfg(windows)]
pub use windows_autostart::{WindowsAutostart, WindowsAutostartError, WindowsAutostartStatus};
pub use windows_control::{
    WINDOWS_FOREGROUND_INGRESS_CAPACITY, WindowsControlAdapter, WindowsControlDrain,
};
#[cfg(windows)]
pub use windows_control::{
    WindowsApplicationMetadataRequest, WindowsControlError, WindowsControlWindow,
    WindowsWindowTitleObservationRequest,
};
#[cfg(windows)]
pub use windows_metadata::extract_application_icon;
pub use windows_single_instance::{
    ActivationCommandDecode, LocalActivationCommand, WINDOWS_ACTIVATION_MESSAGE_LIMIT,
};
#[cfg(windows)]
pub use windows_single_instance::{
    ActivationSendOutcome, InstanceAcquisition, WindowsActivationServer, WindowsExistingInstance,
    WindowsInstanceError, WindowsInstanceOwner,
};
pub use windows_tray::{CloseToTrayDisposition, WindowsPlatformAction, WindowsTrayController};
#[cfg(windows)]
pub use windows_tray::{WindowsTray, WindowsTrayError};
