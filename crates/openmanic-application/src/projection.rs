//! Immutable projection requests, snapshots, and stale-result acceptance rules.

use core::fmt;
use std::sync::Arc;

use crate::{DataRevision, ProjectionContextKey, ProjectionSlot, RequestId, SchemaRevision};

/// Requests one presentation-ready projection from the background application layer.
///
/// `K` is a typed projection kind defined by the use case that owns the
/// calculation. It must contain the normalized range, filter, selection, and
/// configuration inputs represented by [`ProjectionContextKey`].
#[derive(Debug, Eq, PartialEq)]
pub struct ProjectionRequest<K> {
    request_id: RequestId,
    slot: ProjectionSlot,
    context_key: ProjectionContextKey,
    required_data_revision: DataRevision,
    payload: K,
}

impl<K> ProjectionRequest<K> {
    /// Creates a projection request with correlation and minimum-revision metadata.
    #[must_use]
    pub const fn new(
        request_id: RequestId,
        slot: ProjectionSlot,
        context_key: ProjectionContextKey,
        required_data_revision: DataRevision,
        payload: K,
    ) -> Self {
        Self {
            request_id,
            slot,
            context_key,
            required_data_revision,
            payload,
        }
    }

    /// Returns the request identifier used to reject superseded results.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the target projection slot.
    #[must_use]
    pub const fn slot(&self) -> ProjectionSlot {
        self.slot
    }

    /// Returns the normalized context associated with this request.
    #[must_use]
    pub const fn context_key(&self) -> ProjectionContextKey {
        self.context_key
    }

    /// Returns the minimum committed store revision required by the requester.
    #[must_use]
    pub const fn required_data_revision(&self) -> DataRevision {
        self.required_data_revision
    }

    /// Borrows the typed projection payload.
    #[must_use]
    pub const fn payload(&self) -> &K {
        &self.payload
    }

    /// Consumes the request and returns its typed projection payload.
    #[must_use]
    pub fn into_payload(self) -> K {
        self.payload
    }
}

/// An immutable projection value published with correlation and source metadata.
///
/// The envelope exposes only shared or borrowed access to `T`. Producers must
/// therefore fully prepare the value before publication; renderers receive a
/// read model rather than authoritative writable state.
#[derive(Clone, Debug)]
pub struct SnapshotEnvelope<T> {
    request_id: RequestId,
    slot: ProjectionSlot,
    context_key: ProjectionContextKey,
    source_data_revision: DataRevision,
    snapshot_schema_revision: SchemaRevision,
    value: Arc<T>,
}

impl<T> SnapshotEnvelope<T> {
    /// Creates an immutable snapshot envelope from an owned presentation value.
    #[must_use]
    pub fn new(
        request_id: RequestId,
        slot: ProjectionSlot,
        context_key: ProjectionContextKey,
        source_data_revision: DataRevision,
        snapshot_schema_revision: SchemaRevision,
        value: T,
    ) -> Self {
        Self::from_shared(
            request_id,
            slot,
            context_key,
            source_data_revision,
            snapshot_schema_revision,
            Arc::new(value),
        )
    }

    /// Creates an immutable snapshot envelope from an already shared value.
    #[must_use]
    pub const fn from_shared(
        request_id: RequestId,
        slot: ProjectionSlot,
        context_key: ProjectionContextKey,
        source_data_revision: DataRevision,
        snapshot_schema_revision: SchemaRevision,
        value: Arc<T>,
    ) -> Self {
        Self {
            request_id,
            slot,
            context_key,
            source_data_revision,
            snapshot_schema_revision,
            value,
        }
    }

    /// Returns the request identifier this snapshot answers.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the projection slot this snapshot targets.
    #[must_use]
    pub const fn slot(&self) -> ProjectionSlot {
        self.slot
    }

    /// Returns the normalized context used to produce this snapshot.
    #[must_use]
    pub const fn context_key(&self) -> ProjectionContextKey {
        self.context_key
    }

    /// Returns the committed store revision read by the projection producer.
    #[must_use]
    pub const fn source_data_revision(&self) -> DataRevision {
        self.source_data_revision
    }

    /// Returns the version of this snapshot's serialized shape.
    #[must_use]
    pub const fn snapshot_schema_revision(&self) -> SchemaRevision {
        self.snapshot_schema_revision
    }

    /// Borrows the immutable presentation-ready value.
    #[must_use]
    pub fn value(&self) -> &T {
        self.value.as_ref()
    }

    /// Clones the shared immutable presentation-ready value.
    #[must_use]
    pub fn shared_value(&self) -> Arc<T> {
        Arc::clone(&self.value)
    }
}

/// Tracks the current request and latest accepted revision for one projection slot.
///
/// The owner is normally the UI controller. It may cancel superseded work for
/// efficiency, but [`Self::accept_if_current`] remains the correctness mechanism
/// when a late result still arrives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionSlotState {
    slot: ProjectionSlot,
    current: Option<CurrentProjection>,
    last_accepted_data_revision: Option<DataRevision>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CurrentProjection {
    request_id: RequestId,
    context_key: ProjectionContextKey,
    required_data_revision: DataRevision,
}

impl ProjectionSlotState {
    /// Begins tracking results for the supplied current request.
    ///
    /// A producer cannot satisfy the request with source data older than its
    /// required revision. No snapshot has been accepted at construction time.
    #[must_use]
    pub fn new<K>(request: &ProjectionRequest<K>) -> Self {
        Self {
            slot: request.slot(),
            current: Some(CurrentProjection {
                request_id: request.request_id(),
                context_key: request.context_key(),
                required_data_revision: request.required_data_revision(),
            }),
            last_accepted_data_revision: None,
        }
    }

    /// Returns the stable slot controlled by this state.
    #[must_use]
    pub const fn slot(&self) -> ProjectionSlot {
        self.slot
    }

    /// Returns whether the target slot or widget still exists.
    #[must_use]
    pub const fn target_exists(&self) -> bool {
        self.current.is_some()
    }

    /// Returns the current request identifier when the target still exists.
    #[must_use]
    pub const fn current_request_id(&self) -> Option<RequestId> {
        match self.current {
            Some(current) => Some(current.request_id),
            None => None,
        }
    }

    /// Returns the source revision of the newest accepted snapshot, if any.
    #[must_use]
    pub const fn last_accepted_data_revision(&self) -> Option<DataRevision> {
        self.last_accepted_data_revision
    }

    /// Makes a new request current for this existing projection slot.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRejection::SlotMismatch`] if the request targets a
    /// different slot, or [`SnapshotRejection::TargetMissing`] if this target
    /// was removed and must be recreated instead.
    pub fn replace_current_request<K>(
        &mut self,
        request: &ProjectionRequest<K>,
    ) -> Result<(), SnapshotRejection> {
        if self.current.is_none() {
            return Err(SnapshotRejection::TargetMissing { slot: self.slot });
        }
        if request.slot() != self.slot {
            return Err(SnapshotRejection::SlotMismatch {
                expected: self.slot,
                received: request.slot(),
            });
        }

        self.current = Some(CurrentProjection {
            request_id: request.request_id(),
            context_key: request.context_key(),
            required_data_revision: request.required_data_revision(),
        });
        Ok(())
    }

    /// Marks the target slot or widget as removed.
    pub fn remove_target(&mut self) {
        self.current = None;
    }

    /// Accepts a snapshot only when it is current, contextual, and non-stale.
    ///
    /// # Errors
    ///
    /// Returns a [`SnapshotRejection`] when the target has been removed, the
    /// slot, request, or context is stale, or the snapshot source revision is
    /// older than the request requirement or newest accepted revision. On
    /// success, the snapshot's source revision becomes the newest accepted
    /// revision.
    pub fn accept_if_current<T>(
        &mut self,
        snapshot: &SnapshotEnvelope<T>,
    ) -> Result<(), SnapshotRejection> {
        let Some(current) = self.current else {
            return Err(SnapshotRejection::TargetMissing { slot: self.slot });
        };
        if snapshot.slot() != self.slot {
            return Err(SnapshotRejection::SlotMismatch {
                expected: self.slot,
                received: snapshot.slot(),
            });
        }
        if snapshot.request_id() != current.request_id {
            return Err(SnapshotRejection::RequestNotCurrent {
                expected: current.request_id,
                received: snapshot.request_id(),
            });
        }
        if snapshot.context_key() != current.context_key {
            return Err(SnapshotRejection::ContextMismatch {
                expected: current.context_key,
                received: snapshot.context_key(),
            });
        }
        if snapshot.source_data_revision() < current.required_data_revision {
            return Err(SnapshotRejection::SourceRevisionOlderThanRequired {
                source: snapshot.source_data_revision(),
                required: current.required_data_revision,
            });
        }
        if let Some(last_accepted) = self.last_accepted_data_revision
            && snapshot.source_data_revision() < last_accepted
        {
            return Err(SnapshotRejection::SourceRevisionOlderThanAccepted {
                source: snapshot.source_data_revision(),
                last_accepted,
            });
        }

        self.last_accepted_data_revision = Some(snapshot.source_data_revision());
        Ok(())
    }
}

/// Explains why a projection result must not replace the current snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotRejection {
    /// The receiving slot or widget has been removed.
    TargetMissing {
        /// The removed target slot.
        slot: ProjectionSlot,
    },
    /// The snapshot or request targeted a different slot.
    SlotMismatch {
        /// The receiving slot expected by the controller.
        expected: ProjectionSlot,
        /// The slot carried by the request or snapshot.
        received: ProjectionSlot,
    },
    /// The snapshot answered a request superseded by a newer request for the slot.
    RequestNotCurrent {
        /// The current request identifier.
        expected: RequestId,
        /// The stale request identifier carried by the snapshot.
        received: RequestId,
    },
    /// The snapshot was computed for a different normalized projection context.
    ContextMismatch {
        /// The current normalized context key.
        expected: ProjectionContextKey,
        /// The mismatched context key carried by the snapshot.
        received: ProjectionContextKey,
    },
    /// The snapshot source data is older than the request's required revision.
    SourceRevisionOlderThanRequired {
        /// The source revision carried by the stale snapshot.
        source: DataRevision,
        /// The minimum revision required by the current request.
        required: DataRevision,
    },
    /// The snapshot source data is older than an already accepted result.
    SourceRevisionOlderThanAccepted {
        /// The source revision carried by the stale snapshot.
        source: DataRevision,
        /// The newest revision already accepted for this slot.
        last_accepted: DataRevision,
    },
}

impl fmt::Display for SnapshotRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetMissing { slot } => {
                write!(formatter, "projection slot {} no longer exists", slot.get())
            }
            Self::SlotMismatch { expected, received } => write!(
                formatter,
                "projection slot mismatch: expected {}, received {}",
                expected.get(),
                received.get()
            ),
            Self::RequestNotCurrent { expected, received } => write!(
                formatter,
                "projection request is stale: expected {}, received {}",
                expected.get(),
                received.get()
            ),
            Self::ContextMismatch { expected, received } => write!(
                formatter,
                "projection context mismatch: expected {}, received {}",
                expected.get(),
                received.get()
            ),
            Self::SourceRevisionOlderThanRequired { source, required } => write!(
                formatter,
                "projection source revision {} is older than required revision {}",
                source.get(),
                required.get()
            ),
            Self::SourceRevisionOlderThanAccepted {
                source,
                last_accepted,
            } => write!(
                formatter,
                "projection source revision {} is older than accepted revision {}",
                source.get(),
                last_accepted.get()
            ),
        }
    }
}

impl std::error::Error for SnapshotRejection {}
