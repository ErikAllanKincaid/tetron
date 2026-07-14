//! Shared wire protocol for tetron.
//!
//! Both the `tetron` daemon/CLI and GUI frontends speak the same [`ipc::IpcMessage`]
//! enum over a length-prefixed msgpack Unix socket. This crate is the single source
//! of truth for that protocol so frontends never hand-mirror it.

pub mod firewall;
pub mod ipc;
pub mod policy;
mod types;

pub use firewall::{Action, Direction, Protocol};
pub use policy::{HostSuggestions, SuggestedFirewall};
pub use types::{GroupMode, TransportMode};
