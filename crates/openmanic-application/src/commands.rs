//! Typed command envelopes shared by application use cases.

use openmanic_domain::UtcMicros;

use crate::{CommandId, EntityRevision, OrderingKey, SchemaRevision};

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
