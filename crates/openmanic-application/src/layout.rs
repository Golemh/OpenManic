//! Durable Today-dashboard layout snapshots and the sole persistence boundary.

use openmanic_domain::{LayoutDocument, UtcMicros};

use crate::{DataRevision, EntityRevision};

/// Immutable persisted dashboard layout with its optimistic revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutSnapshot {
    document: LayoutDocument,
    entity_revision: EntityRevision,
    updated_at_utc: UtcMicros,
}

impl LayoutSnapshot {
    /// Creates a snapshot from a validated layout document and authoritative metadata.
    #[must_use]
    pub const fn new(
        document: LayoutDocument,
        entity_revision: EntityRevision,
        updated_at_utc: UtcMicros,
    ) -> Self {
        Self {
            document,
            entity_revision,
            updated_at_utc,
        }
    }

    /// Returns the complete validated layout document.
    #[must_use]
    pub const fn document(&self) -> &LayoutDocument {
        &self.document
    }

    /// Returns the optimistic revision required to replace this layout.
    #[must_use]
    pub const fn entity_revision(&self) -> EntityRevision {
        self.entity_revision
    }

    /// Returns the authoritative update instant.
    #[must_use]
    pub const fn updated_at_utc(&self) -> UtcMicros {
        self.updated_at_utc
    }
}

/// Storage failures exposed by the durable layout boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutPersistenceError {
    /// The layout is absent or its revision no longer matches.
    RevisionConflict,
    /// The stored document cannot be decoded as a valid layout.
    InvalidDocument,
    /// The adapter failed before committing a replacement.
    Failed,
}

/// Authoritative storage operations for the singleton dashboard layout.
pub trait LayoutPersistence {
    /// Loads the current durable layout snapshot, if one has been saved.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutPersistenceError`] when the layout cannot be decoded or read.
    fn load_layout(&mut self) -> Result<Option<LayoutSnapshot>, LayoutPersistenceError>;

    /// Atomically saves a replacement after checking the expected revision.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutPersistenceError`] when optimistic concurrency or persistence fails.
    fn replace_layout(
        &mut self,
        snapshot: &LayoutSnapshot,
        expected_revision: Option<EntityRevision>,
    ) -> Result<DataRevision, LayoutPersistenceError>;
}

/// Result of an attempted layout replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutMutation {
    /// The replacement committed at this store revision.
    Confirmed(DataRevision),
    /// The replacement was rejected without changing the stored layout.
    Rejected(LayoutPersistenceError),
}

/// Applies complete layout replacements through the sole persistence authority.
pub struct LayoutService<P> {
    persistence: P,
}

impl<P> LayoutService<P>
where
    P: LayoutPersistence,
{
    /// Creates a service around its exclusive persistence port.
    #[must_use]
    pub const fn new(persistence: P) -> Self {
        Self { persistence }
    }

    /// Loads the saved layout without synthesizing persistence results.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutPersistenceError`] when the storage adapter cannot load it.
    pub fn load(&mut self) -> Result<Option<LayoutSnapshot>, LayoutPersistenceError> {
        self.persistence.load_layout()
    }

    /// Saves one complete validated layout document at an authoritative observation time.
    #[must_use]
    pub fn save(
        &mut self,
        document: LayoutDocument,
        expected_revision: Option<EntityRevision>,
        observed_at_utc: UtcMicros,
    ) -> LayoutMutation {
        let snapshot = LayoutSnapshot::new(
            document,
            expected_revision.unwrap_or(EntityRevision::new(0)),
            observed_at_utc,
        );
        self.persistence
            .replace_layout(&snapshot, expected_revision)
            .map_or_else(LayoutMutation::Rejected, LayoutMutation::Confirmed)
    }
}
