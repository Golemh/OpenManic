//! Stable application-boundary identifier value types.

macro_rules! define_u64_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[doc = "\n\nThe value shape is an unsigned 64-bit integer. It is deliberately"]
        #[doc = "not interchangeable with other application identifiers."]
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            /// Creates an identifier from its persisted unsigned integer value.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the exact unsigned integer value for a serialization boundary.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

define_u64_id!(
    CommandId,
    "Identifies one submitted command and correlates its eventual outcome."
);
define_u64_id!(
    JobId,
    "Identifies one background operation across its progress and final event."
);
define_u64_id!(
    RequestId,
    "Identifies one projection request for stale-result correlation."
);
define_u64_id!(
    DataRevision,
    "Identifies a monotonically increasing committed store revision."
);
define_u64_id!(
    EntityRevision,
    "Identifies the revision expected for an optimistic entity mutation."
);
define_u64_id!(
    OrderingKey,
    "Identifies the entity or service whose commands require a shared order."
);
define_u64_id!(
    ProjectionSlot,
    "Identifies the stable UI location that owns one projection result."
);
define_u64_id!(
    ProjectionContextKey,
    "Identifies normalized projection context such as range, filters, and configuration."
);

/// Identifies the version of a serialized command, event, or snapshot shape.
///
/// The value shape is an unsigned 16-bit integer so schema versions remain
/// distinct from command, request, and store-revision identifiers.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SchemaRevision(u16);

impl SchemaRevision {
    /// Creates a schema revision from its exact unsigned integer value.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the exact unsigned integer value for a serialization boundary.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}
