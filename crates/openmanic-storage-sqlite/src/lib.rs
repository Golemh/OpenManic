//! SQLite implementations of OpenManic application-layer persistence ports.
//!
//! This crate owns future connections, transactions, migrations, and row mappings. It does not
//! own product policy or presentation, and it must not expose concrete SQLite types across its
//! public boundary. Persistence will use one serialized writer and short read transactions.

#![forbid(unsafe_code)]

mod connection;
mod errors;
mod migration;
mod options;

pub use connection::{
    ConnectionConfiguration, JournalMode, SqliteReader, SqliteWriter, SynchronousMode,
};
pub use errors::{ConnectionSetting, StorageError};
pub use migration::LATEST_SCHEMA_VERSION;
pub use options::StoreOpenOptions;
