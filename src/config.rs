//! Config: on-disk schema, `config set`/`config get` override resolution, and
//! filesystem persistence — split into `config/schema.rs`, `config/
//! overrides.rs`, `config/storage.rs` (MODULARIZE-003/004). This file is a
//! re-export shim only, so every existing `crate::config::…` path keeps
//! compiling unchanged (same pattern already proven with `GroupMode` in
//! `membership.rs`, and with `TransportMode` immediately below, already
//! re-exported here before this split existed).

mod overrides;
mod schema;
mod storage;

pub use overrides::*;
pub use schema::*;
pub use storage::*;

#[cfg(test)]
pub(crate) use storage::CONFIG_ENV_LOCK;

/// Per-network transport preference. Defined in `tetron-proto` (shared with GUI
/// frontends); re-exported here so existing `crate::config::TransportMode` paths work.
pub use tetron_proto::TransportMode;
