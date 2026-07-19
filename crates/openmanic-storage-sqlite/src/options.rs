//! Explicit writer-open metadata for initializing a fresh local store.

/// Metadata supplied when a writer opens a local SQLite store.
///
/// The stable store identity is inserted only when the database is first
/// initialized. Later opens verify it, while the application version updates
/// diagnostic metadata without changing the identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreOpenOptions {
    store_id: [u8; 16],
    opened_utc_us: i64,
    app_version: String,
}

impl StoreOpenOptions {
    /// Creates metadata used to initialize or verify a local SQLite store.
    #[must_use]
    pub fn new(store_id: [u8; 16], opened_utc_us: i64, app_version: impl Into<String>) -> Self {
        Self {
            store_id,
            opened_utc_us,
            app_version: app_version.into(),
        }
    }

    /// Returns the stable 16-byte identity for the requested store.
    #[must_use]
    pub const fn store_id(&self) -> [u8; 16] {
        self.store_id
    }

    /// Returns the signed UTC-microsecond timestamp for this open operation.
    #[must_use]
    pub const fn opened_utc_us(&self) -> i64 {
        self.opened_utc_us
    }

    /// Returns the application version recorded in store metadata and the migration ledger.
    #[must_use]
    pub fn app_version(&self) -> &str {
        &self.app_version
    }
}
