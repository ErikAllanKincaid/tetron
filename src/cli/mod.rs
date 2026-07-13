//! CLI command handlers, split by domain. `main.rs` (the binary crate root)
//! holds the clap definitions, `main` dispatch, tracing/panic plumbing, and the
//! shared presentation helpers; the per-command handlers live here.
//!
//! Each submodule opens with `use crate::*;` to inherit the crate-root imports
//! and helpers, and this module flattens them back out with `pub use <m>::*;`.
//! `main.rs` then does `use cli::*;`, so every handler — in root or any
//! submodule — resolves the others through the crate-root namespace. Submodules
//! are kept private (`mod`, not `pub mod`) so only their *contents* are
//! re-exported, avoiding a name clash with the `use tetron::{invite, …}`
//! aliases in the crate root.

mod admin;
mod invite;
mod network;
mod service;
mod status;

pub(crate) use admin::*;
pub(crate) use invite::*;
pub(crate) use network::*;
pub(crate) use service::*;
pub(crate) use status::*;
