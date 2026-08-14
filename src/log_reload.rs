//! Live-reloadable file-log level (LOG-004, follow-up to LOG-003).
//!
//! `main::init_tracing()` wraps the file layer's `EnvFilter` in a
//! `tracing_subscriber::reload::Layer` and registers the returned `Handle`
//! here via [`set_handle`] -- process-global state, the same shape
//! `LogGuard`/the panic hook already use for tracing/process-lifetime
//! concerns, since there is exactly one subscriber per process. A daemon
//! handling `IpcMessage::SetLogLevel` then calls [`reload_log_level`] to
//! swap the running filter without a restart.

use std::sync::OnceLock;

use anyhow::Result;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Registry;
use tracing_subscriber::reload;

type Handle = reload::Handle<EnvFilter, Registry>;

static HANDLE: OnceLock<Handle> = OnceLock::new();

/// Registers the reload handle. Called once from `init_tracing()` at
/// process startup; a second call (there should never be one) is a no-op.
pub fn set_handle(handle: Handle) {
    let _ = HANDLE.set(handle);
}

/// Builds the same `"info,tetron={level}"` filter directive `init_tracing()`
/// computes at startup when `RUST_LOG` is unset.
fn build_filter(level: &str) -> Result<EnvFilter> {
    EnvFilter::try_new(format!("info,tetron={level}"))
        .map_err(|e| anyhow::anyhow!("invalid log level {level:?}: {e}"))
}

/// Swaps the running file-log filter to `level` (trace/debug/info/warn/error).
/// Errors if no handle was ever registered (a process that never called
/// `init_tracing(true)`, e.g. a bare CLI invocation) or if the reload itself
/// fails (the subscriber was somehow dropped).
pub fn reload_log_level(level: &str) -> Result<()> {
    let handle = HANDLE
        .get()
        .ok_or_else(|| anyhow::anyhow!("log filter is not live-reloadable in this process"))?;
    let filter = build_filter(level)?;
    handle
        .reload(filter)
        .map_err(|e| anyhow::anyhow!("failed to reload log filter: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_filter_accepts_every_valid_level() {
        for level in ["trace", "debug", "info", "warn", "error"] {
            assert!(build_filter(level).is_ok(), "level {level} should parse");
        }
    }

    #[test]
    fn build_filter_rejects_garbage() {
        assert!(build_filter("not-a-level!!").is_err());
    }

    /// No test in this binary ever calls `set_handle` (only `main::init_tracing`
    /// does, and tests never invoke `main`), so the handle is guaranteed unset
    /// for the lifetime of this process -- safe to assert on deterministically.
    #[test]
    fn reload_log_level_errors_when_handle_not_set() {
        assert!(reload_log_level("debug").is_err());
    }
}
