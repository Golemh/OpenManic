//! Typed command envelopes shared by application use cases.

use openmanic_domain::{ApplicationId, PowerTransitionEvidence, UtcMicros};

use crate::{CommandId, EntityRevision, OrderingKey, SchemaRevision};

/// A command handled by the platform-neutral tracking service.
///
/// Platform adapters submit [`Self::Evidence`] after normalizing operating-system
/// observations. The UI submits the explicit pause, resume, and checkpoint commands.
/// Every timestamp is supplied by the envelope or evidence so the service does not
/// depend on a wall clock or a repaint cadence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingCommand {
    /// Records an explicitly requested user pause at the envelope timestamp.
    Pause,
    /// Ends an explicit user pause at the envelope timestamp.
    Resume,
    /// Requests a five-second-policy checkpoint at the envelope timestamp.
    Checkpoint,
    /// Applies one normalized platform observation.
    Evidence(TrackingEvidence),
}

/// A normalized, ordered observation consumed by the tracking reducer.
///
/// `sequence` is monotonic for one platform adapter. Duplicate or lower sequence
/// values are ignored rather than being allowed to fabricate or overlap activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackingEvidence {
    /// A resolved foreground application observation.
    Foreground {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the application was observed.
        observed_at_utc: UtcMicros,
        /// Resolved stable application identity.
        application_id: ApplicationId,
    },
    /// A foreground observation that matches the user's exclusion policy.
    ExcludedForeground {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the excluded foreground was observed.
        observed_at_utc: UtcMicros,
    },
    /// The configured idle threshold was crossed.
    IdleThresholdCrossed {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the threshold crossing was observed.
        observed_at_utc: UtcMicros,
    },
    /// The workstation was locked.
    SessionLocked {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the lock was observed.
        observed_at_utc: UtcMicros,
    },
    /// The user session disconnected.
    SessionDisconnected {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the disconnect was observed.
        observed_at_utc: UtcMicros,
    },
    /// The system suspended or hibernated.
    SystemSuspended {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which suspension was observed.
        observed_at_utc: UtcMicros,
    },
    /// The adapter has started but has not yet established trusted foreground evidence.
    AdapterStarting {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which adapter startup was observed.
        observed_at_utc: UtcMicros,
    },
    /// The adapter lost a permission required for trustworthy tracking.
    AdapterPermissionLost {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which permission loss was observed.
        observed_at_utc: UtcMicros,
    },
    /// The adapter failed without a more specific permission explanation.
    AdapterFailure {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the failure was observed.
        observed_at_utc: UtcMicros,
    },
    /// A bounded adapter ingress lost evidence and requires honest uncertainty.
    EvidenceQueueOverflow {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the loss was reported.
        observed_at_utc: UtcMicros,
    },
    /// A wall-clock discontinuity makes normal attribution uncertain.
    ClockDiscontinuity {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// UTC instant at which the discontinuity was reported.
        observed_at_utc: UtcMicros,
    },
    /// Affirmative shutdown and later startup boundaries.
    ConfirmedPowerTransition {
        /// Monotonic platform-adapter sequence.
        sequence: u64,
        /// Qualifying shutdown and startup evidence.
        boundaries: PowerTransitionEvidence,
    },
}

impl TrackingEvidence {
    /// Returns the source-local sequence used to reject duplicate or reordered evidence.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        match self {
            Self::Foreground { sequence, .. }
            | Self::ExcludedForeground { sequence, .. }
            | Self::IdleThresholdCrossed { sequence, .. }
            | Self::SessionLocked { sequence, .. }
            | Self::SessionDisconnected { sequence, .. }
            | Self::SystemSuspended { sequence, .. }
            | Self::AdapterStarting { sequence, .. }
            | Self::AdapterPermissionLost { sequence, .. }
            | Self::AdapterFailure { sequence, .. }
            | Self::EvidenceQueueOverflow { sequence, .. }
            | Self::ClockDiscontinuity { sequence, .. }
            | Self::ConfirmedPowerTransition { sequence, .. } => sequence,
        }
    }

    /// Returns the UTC observation time used to place a canonical interval boundary.
    #[must_use]
    pub const fn observed_at_utc(self) -> UtcMicros {
        match self {
            Self::Foreground {
                observed_at_utc, ..
            }
            | Self::ExcludedForeground {
                observed_at_utc, ..
            }
            | Self::IdleThresholdCrossed {
                observed_at_utc, ..
            }
            | Self::SessionLocked {
                observed_at_utc, ..
            }
            | Self::SessionDisconnected {
                observed_at_utc, ..
            }
            | Self::SystemSuspended {
                observed_at_utc, ..
            }
            | Self::AdapterStarting {
                observed_at_utc, ..
            }
            | Self::AdapterPermissionLost {
                observed_at_utc, ..
            }
            | Self::AdapterFailure {
                observed_at_utc, ..
            }
            | Self::EvidenceQueueOverflow {
                observed_at_utc, ..
            }
            | Self::ClockDiscontinuity {
                observed_at_utc, ..
            } => observed_at_utc,
            Self::ConfirmedPowerTransition { boundaries, .. } => boundaries.startup(),
        }
    }
}

/// A submitted command with stable correlation and ordering metadata.
///
/// `P` is an enumerated command payload owned by the use case that validates
/// and handles it. The envelope owns no storage, platform, or UI behavior.
#[derive(Debug, Eq, PartialEq)]
pub struct CommandEnvelope<P> {
    schema_revision: SchemaRevision,
    command_id: CommandId,
    ordering_key: OrderingKey,
    expected_entity_revision: Option<EntityRevision>,
    submitted_at_utc: UtcMicros,
    payload: P,
}

impl<P> CommandEnvelope<P> {
    /// Creates a command envelope with all correlation metadata supplied by its caller.
    #[must_use]
    pub const fn new(
        schema_revision: SchemaRevision,
        command_id: CommandId,
        ordering_key: OrderingKey,
        expected_entity_revision: Option<EntityRevision>,
        submitted_at_utc: UtcMicros,
        payload: P,
    ) -> Self {
        Self {
            schema_revision,
            command_id,
            ordering_key,
            expected_entity_revision,
            submitted_at_utc,
            payload,
        }
    }

    /// Returns the command-shape schema revision.
    #[must_use]
    pub const fn schema_revision(&self) -> SchemaRevision {
        self.schema_revision
    }

    /// Returns the identifier that correlates this command with later events.
    #[must_use]
    pub const fn command_id(&self) -> CommandId {
        self.command_id
    }

    /// Returns the key used to serialize conflicting commands.
    #[must_use]
    pub const fn ordering_key(&self) -> OrderingKey {
        self.ordering_key
    }

    /// Returns the optional entity revision required for optimistic concurrency.
    #[must_use]
    pub const fn expected_entity_revision(&self) -> Option<EntityRevision> {
        self.expected_entity_revision
    }

    /// Returns the authoritative UTC submission timestamp.
    #[must_use]
    pub const fn submitted_at_utc(&self) -> UtcMicros {
        self.submitted_at_utc
    }

    /// Borrows the typed command payload.
    #[must_use]
    pub const fn payload(&self) -> &P {
        &self.payload
    }

    /// Consumes the envelope and returns its typed payload.
    #[must_use]
    pub fn into_payload(self) -> P {
        self.payload
    }
}

/// Acknowledges that a command was accepted for lossless application handling.
///
/// This is not mutation confirmation. Authoritative success or failure arrives
/// later as a [`crate::MutationOutcome`] correlated by the same command ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandReceipt {
    command_id: CommandId,
}

impl CommandReceipt {
    /// Creates a receipt for a command accepted by the application supervisor.
    #[must_use]
    pub const fn accepted(command_id: CommandId) -> Self {
        Self { command_id }
    }

    /// Returns the accepted command identifier.
    #[must_use]
    pub const fn command_id(self) -> CommandId {
        self.command_id
    }
}
