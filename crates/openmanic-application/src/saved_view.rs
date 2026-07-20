//! Saved Overview-view commands, restoration snapshots, and persistence boundary.

use openmanic_domain::{SavedViewDefinition, SavedViewDocument, UtcMicros};

use crate::{DataRevision, EntityRevision};

/// Stable public identity of one saved Overview view.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SavedViewId([u8; 16]);

impl SavedViewId {
    /// Creates an ID from its storage-stable bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the storage-stable bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Returns the lowercase fixed-width serialization used inside view documents.
    #[must_use]
    pub fn encoded(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(self.0.len() * 2);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }
}

/// One immutable saved view restored by the Overview controller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedViewSnapshot {
    id: SavedViewId,
    document: SavedViewDocument,
    entity_revision: EntityRevision,
    created_at_utc: UtcMicros,
}

impl SavedViewSnapshot {
    /// Creates a snapshot only when its stable row identity and document identity agree.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewSnapshotError`] when the serialized document ID differs from the row ID.
    pub fn try_new(
        id: SavedViewId,
        document: SavedViewDocument,
        entity_revision: EntityRevision,
        created_at_utc: UtcMicros,
    ) -> Result<Self, SavedViewSnapshotError> {
        if document.public_id() != id.encoded() {
            return Err(SavedViewSnapshotError::IdentityMismatch);
        }
        Ok(Self {
            id,
            document,
            entity_revision,
            created_at_utc,
        })
    }

    /// Returns the stable identity.
    #[must_use]
    pub const fn id(&self) -> SavedViewId {
        self.id
    }
    /// Returns the fully validated saved-view restoration document.
    #[must_use]
    pub fn document(&self) -> &SavedViewDocument {
        &self.document
    }
    /// Returns the optimistic entity revision.
    #[must_use]
    pub const fn entity_revision(&self) -> EntityRevision {
        self.entity_revision
    }
    /// Returns the authoritative creation time.
    #[must_use]
    pub const fn created_at_utc(&self) -> UtcMicros {
        self.created_at_utc
    }
}

/// Rejection while constructing a snapshot from persisted values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SavedViewSnapshotError {
    /// Row and document public identities differ.
    IdentityMismatch,
}

/// A requested durable saved-view change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SavedViewCommand {
    /// Creates one new view with an application-generated stable ID.
    Create {
        /// Stable identity assigned to the view.
        id: SavedViewId,
        /// Fully configured controls to persist.
        definition: SavedViewDefinition,
        /// Authoritative observation time recorded for the new view.
        observed_at_utc: UtcMicros,
    },
    /// Renames an existing view without changing its restored controls.
    Rename {
        /// Stable identity of the view to rename.
        id: SavedViewId,
        /// Replacement display name.
        name: String,
        /// Authoritative observation time recorded for the change.
        observed_at_utc: UtcMicros,
    },
    /// Copies one loaded view to an application-generated stable ID.
    Duplicate {
        /// Stable identity of the loaded source view.
        source_id: SavedViewId,
        /// New stable identity assigned to the duplicate.
        duplicate_id: SavedViewId,
        /// Display name assigned to the duplicate.
        name: String,
        /// Authoritative observation time recorded for the duplicate.
        observed_at_utc: UtcMicros,
    },
    /// Replaces the complete display ordering with every current ID exactly once.
    Reorder {
        /// Complete display ordering, containing each current ID exactly once.
        ordered_ids: Vec<SavedViewId>,
    },
    /// Deletes one view only after the caller has obtained explicit confirmation.
    DeleteConfirmed {
        /// Stable identity of the explicitly confirmed deletion target.
        id: SavedViewId,
    },
}

/// Durable operations required by the saved-view application service.
pub trait SavedViewPersistence {
    /// Lists valid snapshots ordered by display order then stable ID, plus invalid rows retained for diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewPersistenceError`] when the store cannot be read.
    fn load_saved_views(&mut self) -> Result<SavedViewLoad, SavedViewPersistenceError>;
    /// Creates a view and returns the store revision.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewPersistenceError`] when validation, optimistic concurrency, or persistence fails.
    fn create_saved_view(
        &mut self,
        snapshot: &SavedViewSnapshot,
    ) -> Result<DataRevision, SavedViewPersistenceError>;
    /// Replaces an existing view after optimistic revision validation.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewPersistenceError`] when validation, optimistic concurrency, or persistence fails.
    fn replace_saved_view(
        &mut self,
        snapshot: &SavedViewSnapshot,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SavedViewPersistenceError>;
    /// Atomically replaces the deterministic order after checking every expected revision.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewPersistenceError`] when any expected revision differs or persistence fails.
    fn reorder_saved_views(
        &mut self,
        ordered: &[(SavedViewId, EntityRevision)],
    ) -> Result<DataRevision, SavedViewPersistenceError>;
    /// Deletes one view after optimistic revision validation.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewPersistenceError`] when the target revision differs or persistence fails.
    fn delete_saved_view(
        &mut self,
        id: SavedViewId,
        expected_revision: EntityRevision,
    ) -> Result<DataRevision, SavedViewPersistenceError>;
}

/// Restoration result which never lets one corrupt row discard other valid saved views.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedViewLoad {
    snapshots: Vec<SavedViewSnapshot>,
    invalid_count: usize,
}

impl SavedViewLoad {
    /// Creates a deterministic load result.
    #[must_use]
    pub fn new(mut snapshots: Vec<SavedViewSnapshot>, invalid_count: usize) -> Self {
        snapshots.sort_by_key(|snapshot| (snapshot.document().display_order(), snapshot.id()));
        Self {
            snapshots,
            invalid_count,
        }
    }
    /// Returns valid views in deterministic presentation order.
    #[must_use]
    pub fn snapshots(&self) -> &[SavedViewSnapshot] {
        &self.snapshots
    }
    /// Returns the number of invalid rows restored through the safe default/quarantine path.
    #[must_use]
    pub const fn invalid_count(&self) -> usize {
        self.invalid_count
    }
}

/// Stable saved-view persistence failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SavedViewPersistenceError {
    /// Target missing or optimistic revision differs.
    RevisionConflict,
    /// View definition failed storage validation.
    InvalidDocument,
    /// The adapter failed without committing.
    Failed,
}

/// Result of one saved-view command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SavedViewMutation {
    /// A mutation committed at this store revision.
    Confirmed(DataRevision),
    /// A command was rejected without mutation.
    Rejected(SavedViewRejection),
}

/// Stable command rejection category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SavedViewRejection {
    /// The command lacked a matching loaded revision or target.
    RevisionConflict,
    /// Input failed document validation.
    Validation,
    /// The requested ordering omitted, duplicated, or added IDs.
    InvalidOrder,
    /// Persistence failed before commit.
    PersistenceFailure,
}

/// Applies saved-view commands through a sole persistence authority.
pub struct SavedViewService<P> {
    persistence: P,
}

impl<P> SavedViewService<P>
where
    P: SavedViewPersistence,
{
    /// Creates a service around its exclusive persistence port.
    #[must_use]
    pub const fn new(persistence: P) -> Self {
        Self { persistence }
    }

    /// Loads views without allowing an invalid persisted document to prevent safe restoration.
    ///
    /// # Errors
    ///
    /// Returns [`SavedViewPersistenceError`] when the store cannot be read.
    pub fn load(&mut self) -> Result<SavedViewLoad, SavedViewPersistenceError> {
        self.persistence.load_saved_views()
    }

    /// Executes one mutation against the current persisted state.
    #[must_use]
    pub fn handle(
        &mut self,
        command: SavedViewCommand,
        expected_revision: Option<EntityRevision>,
    ) -> SavedViewMutation {
        match command {
            SavedViewCommand::Create {
                id,
                mut definition,
                observed_at_utc,
            } => {
                definition.public_id = id.encoded();
                let result = SavedViewDocument::try_new(definition, 0)
                    .map_err(|_| SavedViewRejection::Validation)
                    .and_then(|document| {
                        SavedViewSnapshot::try_new(
                            id,
                            document,
                            EntityRevision::new(0),
                            observed_at_utc,
                        )
                        .map_err(|_| SavedViewRejection::Validation)
                    })
                    .and_then(|snapshot| {
                        self.persistence
                            .create_saved_view(&snapshot)
                            .map_err(persistence_rejection)
                    });
                result.map_or_else(SavedViewMutation::Rejected, SavedViewMutation::Confirmed)
            }
            SavedViewCommand::Rename {
                id,
                name,
                observed_at_utc,
            } => self.replace_loaded(id, expected_revision, observed_at_utc, |definition| {
                definition.name = name;
            }),
            SavedViewCommand::Duplicate {
                source_id,
                duplicate_id,
                name,
                observed_at_utc,
            } => {
                let loaded = self.load_loaded(source_id, expected_revision);
                let result = loaded.and_then(|snapshot| {
                    let mut definition = snapshot.document().definition();
                    definition.public_id = duplicate_id.encoded();
                    definition.name = name;
                    let document = SavedViewDocument::try_new(definition, 0)
                        .map_err(|_| SavedViewRejection::Validation)?;
                    let duplicate = SavedViewSnapshot::try_new(
                        duplicate_id,
                        document,
                        EntityRevision::new(0),
                        observed_at_utc,
                    )
                    .map_err(|_| SavedViewRejection::Validation)?;
                    self.persistence
                        .create_saved_view(&duplicate)
                        .map_err(persistence_rejection)
                });
                result.map_or_else(SavedViewMutation::Rejected, SavedViewMutation::Confirmed)
            }
            SavedViewCommand::Reorder { ordered_ids } => self.reorder(&ordered_ids),
            SavedViewCommand::DeleteConfirmed { id } => {
                let Some(expected_revision) = expected_revision else {
                    return SavedViewMutation::Rejected(SavedViewRejection::RevisionConflict);
                };
                self.persistence
                    .delete_saved_view(id, expected_revision)
                    .map_or_else(
                        |error| SavedViewMutation::Rejected(persistence_rejection(error)),
                        SavedViewMutation::Confirmed,
                    )
            }
        }
    }

    fn replace_loaded(
        &mut self,
        id: SavedViewId,
        expected_revision: Option<EntityRevision>,
        observed_at_utc: UtcMicros,
        update: impl FnOnce(&mut SavedViewDefinition),
    ) -> SavedViewMutation {
        let result = self.load_loaded(id, expected_revision).and_then(|loaded| {
            let mut definition = loaded.document().definition();
            update(&mut definition);
            SavedViewDocument::try_new(definition, loaded.entity_revision().get())
                .map_err(|_| SavedViewRejection::Validation)
                .and_then(|document| {
                    SavedViewSnapshot::try_new(
                        id,
                        document,
                        loaded.entity_revision(),
                        observed_at_utc,
                    )
                    .map_err(|_| SavedViewRejection::Validation)
                })
                .and_then(|snapshot| {
                    self.persistence
                        .replace_saved_view(&snapshot, loaded.entity_revision())
                        .map_err(persistence_rejection)
                })
        });
        result.map_or_else(SavedViewMutation::Rejected, SavedViewMutation::Confirmed)
    }

    fn load_loaded(
        &mut self,
        id: SavedViewId,
        expected: Option<EntityRevision>,
    ) -> Result<SavedViewSnapshot, SavedViewRejection> {
        let expected = expected.ok_or(SavedViewRejection::RevisionConflict)?;
        self.persistence
            .load_saved_views()
            .map_err(persistence_rejection)?
            .snapshots()
            .iter()
            .find(|snapshot| snapshot.id() == id && snapshot.entity_revision() == expected)
            .cloned()
            .ok_or(SavedViewRejection::RevisionConflict)
    }

    fn reorder(&mut self, ordered_ids: &[SavedViewId]) -> SavedViewMutation {
        let loaded = match self.persistence.load_saved_views() {
            Ok(loaded) => loaded,
            Err(error) => return SavedViewMutation::Rejected(persistence_rejection(error)),
        };
        let expected = loaded
            .snapshots()
            .iter()
            .map(SavedViewSnapshot::id)
            .collect::<std::collections::BTreeSet<_>>();
        let supplied = ordered_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if expected != supplied || ordered_ids.len() != expected.len() {
            return SavedViewMutation::Rejected(SavedViewRejection::InvalidOrder);
        }
        let revisions = ordered_ids
            .iter()
            .map(|id| {
                loaded
                    .snapshots()
                    .iter()
                    .find(|snapshot| snapshot.id() == *id)
                    .map(|snapshot| (*id, snapshot.entity_revision()))
            })
            .collect::<Option<Vec<_>>>();
        let Some(revisions) = revisions else {
            return SavedViewMutation::Rejected(SavedViewRejection::InvalidOrder);
        };
        self.persistence
            .reorder_saved_views(&revisions)
            .map_or_else(
                |error| SavedViewMutation::Rejected(persistence_rejection(error)),
                SavedViewMutation::Confirmed,
            )
    }
}

fn persistence_rejection(error: SavedViewPersistenceError) -> SavedViewRejection {
    match error {
        SavedViewPersistenceError::RevisionConflict => SavedViewRejection::RevisionConflict,
        SavedViewPersistenceError::InvalidDocument => SavedViewRejection::Validation,
        SavedViewPersistenceError::Failed => SavedViewRejection::PersistenceFailure,
    }
}
