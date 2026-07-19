//! Capability and availability vocabulary that contains platform details at the adapter edge.

/// A discrete platform capability whose absence can degrade an otherwise usable adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum Capability {
    /// The adapter can observe the desktop foreground.
    ForegroundTracking,
    /// The adapter can resolve a stable application identity.
    ApplicationIdentity,
    /// The adapter can distinguish window instances.
    WindowInstance,
    /// The adapter can observe window titles when the user enables collection.
    WindowTitle,
    /// The adapter can identify a process instance without relying on a PID alone.
    ProcessIdentity,
    /// The adapter can resolve an executable path.
    ExecutablePath,
    /// The adapter can observe idle state.
    Idle,
    /// The adapter can observe session lock and disconnect transitions.
    SessionLock,
    /// The adapter can observe suspend and resume transitions.
    SuspendResume,
    /// The adapter can obtain affirmative shutdown evidence.
    ShutdownEvidence,
    /// The adapter can expose a notification-area tray integration.
    Tray,
    /// The adapter can configure login start.
    Autostart,
    /// The adapter can request local notifications.
    Notifications,
}

impl Capability {
    const COUNT: usize = 13;

    const fn index(self) -> usize {
        self as usize
    }

    const fn bit(self) -> u16 {
        1_u16 << self.index()
    }
}

/// A compact set of missing or degraded capabilities.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CapabilitySet(u16);

impl CapabilitySet {
    /// Creates an empty capability set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Creates a set containing one capability.
    #[must_use]
    pub const fn only(capability: Capability) -> Self {
        Self(capability.bit())
    }

    /// Adds one capability to this set.
    #[must_use]
    pub const fn with(mut self, capability: Capability) -> Self {
        self.0 |= capability.bit();
        self
    }

    /// Returns whether this set contains a capability.
    #[must_use]
    pub const fn contains(self, capability: Capability) -> bool {
        self.0 & capability.bit() != 0
    }

    /// Returns whether the set has no capabilities.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Describes how the adapter delivers changes from the platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryCapability {
    /// The platform supplies event-driven delivery.
    EventDriven,
    /// The adapter uses a bounded polling fallback.
    Polling,
    /// No trustworthy delivery mechanism is available.
    Unavailable,
}

/// Describes the scope of foreground observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusScope {
    /// The adapter observes the desktop-global foreground.
    DesktopGlobal,
    /// The adapter sees only a declared subset of desktop windows.
    Partial,
    /// The adapter cannot observe foreground state.
    Unavailable,
}

/// Describes whether a field can be supplied safely and reliably.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldSupport {
    /// The field is available from the current platform environment.
    Available,
    /// The field is temporarily unavailable.
    Unavailable,
    /// The field needs a user-granted permission.
    PermissionRequired,
    /// The field needs an explicitly installed helper.
    HelperRequired,
    /// The platform does not support the field.
    Unsupported,
}

/// Declares whether a platform helper is needed for the selected adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelperRequirement {
    /// No external helper is needed.
    NotRequired,
    /// A separately installed helper is required before the adapter can operate.
    Required,
}

/// Describes the platform's permission boundary without exposing platform error values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionModel {
    /// The adapter needs no user-granted permission.
    None,
    /// The user can grant or revoke the needed permission.
    UserGrant,
    /// A system policy controls the permission.
    SystemPolicy,
}

/// Capability probe results for one selected platform adapter.
///
/// This value describes actual probe results, rather than inferring support from an operating
/// system name. It contains no raw platform handles or operating-system error values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformCapabilities {
    delivery: DeliveryCapability,
    focus_scope: FocusScope,
    field_support: [FieldSupport; Capability::COUNT],
    helper_requirement: HelperRequirement,
    permission_model: PermissionModel,
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        Self {
            delivery: DeliveryCapability::Unavailable,
            focus_scope: FocusScope::Unavailable,
            field_support: [FieldSupport::Unsupported; Capability::COUNT],
            helper_requirement: HelperRequirement::NotRequired,
            permission_model: PermissionModel::None,
        }
    }
}

impl PlatformCapabilities {
    /// Returns probe results that make no unsupported capability claim.
    #[must_use]
    pub fn unavailable() -> Self {
        Self::default()
    }

    /// Replaces the observed delivery mechanism.
    #[must_use]
    pub const fn with_delivery(mut self, delivery: DeliveryCapability) -> Self {
        self.delivery = delivery;
        self
    }

    /// Replaces the observed foreground scope.
    #[must_use]
    pub const fn with_focus_scope(mut self, focus_scope: FocusScope) -> Self {
        self.focus_scope = focus_scope;
        self
    }

    /// Replaces the support state for one capability.
    #[must_use]
    pub const fn with_field_support(
        mut self,
        capability: Capability,
        support: FieldSupport,
    ) -> Self {
        self.field_support[capability.index()] = support;
        self
    }

    /// Replaces the selected helper requirement.
    #[must_use]
    pub const fn with_helper_requirement(mut self, requirement: HelperRequirement) -> Self {
        self.helper_requirement = requirement;
        self
    }

    /// Replaces the observed permission model.
    #[must_use]
    pub const fn with_permission_model(mut self, permission_model: PermissionModel) -> Self {
        self.permission_model = permission_model;
        self
    }

    /// Returns the observed delivery mechanism.
    #[must_use]
    pub const fn delivery(self) -> DeliveryCapability {
        self.delivery
    }

    /// Returns the observed foreground scope.
    #[must_use]
    pub const fn focus_scope(self) -> FocusScope {
        self.focus_scope
    }

    /// Returns support for one field-level capability.
    #[must_use]
    pub const fn field_support(self, capability: Capability) -> FieldSupport {
        self.field_support[capability.index()]
    }

    /// Returns the selected helper requirement.
    #[must_use]
    pub const fn helper_requirement(self) -> HelperRequirement {
        self.helper_requirement
    }

    /// Returns the observed permission model.
    #[must_use]
    pub const fn permission_model(self) -> PermissionModel {
        self.permission_model
    }
}

/// The safe availability state reported by a platform adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterAvailability {
    /// The adapter is initializing and has not supplied trusted foreground evidence.
    Starting,
    /// The adapter is delivering all required evidence.
    Ready,
    /// The adapter can run but cannot provide each listed capability.
    Degraded {
        /// The capabilities currently missing from the adapter.
        missing: CapabilitySet,
    },
    /// The adapter needs a user-granted permission.
    PermissionRequired,
    /// The adapter needs an external helper.
    HelperRequired,
    /// The selected platform is temporarily unable to provide trustworthy evidence.
    TemporarilyUnavailable,
    /// A bounded ingress or downstream sink lost evidence.
    EvidenceLost,
    /// The adapter encountered a non-permission fatal failure.
    Fatal,
    /// The adapter is intentionally stopping and accepts no new platform callbacks.
    Stopping,
    /// The selected platform has no supported adapter.
    Unsupported,
}

/// One observed availability state transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AvailabilityTransition {
    previous: Option<AdapterAvailability>,
    current: AdapterAvailability,
}

impl AvailabilityTransition {
    pub(crate) const fn new(
        previous: Option<AdapterAvailability>,
        current: AdapterAvailability,
    ) -> Self {
        Self { previous, current }
    }

    /// Returns the prior state, or `None` before the adapter first announced availability.
    #[must_use]
    pub const fn previous(self) -> Option<AdapterAvailability> {
        self.previous
    }

    /// Returns the new adapter availability state.
    #[must_use]
    pub const fn current(self) -> AdapterAvailability {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Capability, CapabilitySet, DeliveryCapability, FieldSupport, FocusScope,
        PlatformCapabilities,
    };

    #[test]
    fn capabilities_preserve_individual_probe_results() {
        let capabilities = PlatformCapabilities::unavailable()
            .with_delivery(DeliveryCapability::EventDriven)
            .with_focus_scope(FocusScope::DesktopGlobal)
            .with_field_support(Capability::ForegroundTracking, FieldSupport::Available);

        assert_eq!(capabilities.delivery(), DeliveryCapability::EventDriven);
        assert_eq!(capabilities.focus_scope(), FocusScope::DesktopGlobal);
        assert_eq!(
            capabilities.field_support(Capability::ForegroundTracking),
            FieldSupport::Available
        );
        assert_eq!(
            capabilities.field_support(Capability::WindowTitle),
            FieldSupport::Unsupported
        );
    }

    #[test]
    fn capability_set_reports_each_missing_capability() {
        let missing = CapabilitySet::only(Capability::Idle).with(Capability::SessionLock);

        assert!(missing.contains(Capability::Idle));
        assert!(missing.contains(Capability::SessionLock));
        assert!(!missing.contains(Capability::Tray));
        assert!(!missing.is_empty());
        assert!(CapabilitySet::empty().is_empty());
    }
}
