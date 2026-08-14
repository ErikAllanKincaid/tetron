//! Generic per-callsite console log rate limiter (LOG-006).
//!
//! `main::init_tracing()` wraps `console_layer` in a [`RateLimitFilter`], so
//! any console (journal) line that starts repeating too fast at one call
//! site is throttled automatically -- instead of needing a fresh
//! incident-driven bespoke fix each time, the way `PATH-DIAG-006`
//! (`forward.rs`'s `path_flap_decision`) and `LOG-005`'s reconnect-log half
//! (`daemon/mesh/join.rs`'s `reconnect_log_decision`) each were. Those two
//! stay as-is -- they encode domain semantics (e.g. "per-peer", "always show
//! a fresh window's first attempt") this generic, callsite-only limiter
//! cannot express -- this is a coarser backstop underneath them, not a
//! replacement.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use tracing::Metadata;
use tracing::callsite::Identifier;
use tracing_subscriber::layer::{Context, Filter};

/// Compiled default for `log-ratelimit.threshold`: events at one callsite
/// within one window before further ones in that same window are
/// suppressed.
pub const RATELIMIT_THRESHOLD: u32 = 5;

/// Compiled default for `log-ratelimit.window`, in seconds.
pub const RATELIMIT_WINDOW_SECS: u64 = 60;

/// Decides whether one callsite's next event should be shown. Pure
/// function, identical shape to `path_flap_decision` (`PATH-DIAG-006`) and
/// `reconnect_log_decision` (`LOG-005`): a fresh window's first event
/// always shows (a callsite that only just started firing must not be
/// silently dropped); further events within the window show while
/// `count <= threshold`, are suppressed once exceeded. Returns
/// `(allow, new_window_start, new_count)`.
fn ratelimit_decision(
    now: Instant,
    window_start: Instant,
    count: u32,
    threshold: u32,
    window: Duration,
) -> (bool, Instant, u32) {
    if now.duration_since(window_start) >= window {
        return (true, now, 1);
    }
    let new_count = count + 1;
    (new_count <= threshold, window_start, new_count)
}

struct CallsiteState {
    window_start: Instant,
    count: u32,
    /// Events suppressed since the last one actually shown at this
    /// callsite -- surfaced (via a raw `eprintln!`, see below) the next
    /// time this callsite is allowed through, so a burst is never silently
    /// unaccounted for.
    suppressed_since_shown: u32,
}

/// Process-global rate-limit state, keyed by callsite. Cardinality is
/// bounded by the binary's fixed number of `tracing` macro invocation
/// sites (a compile-time constant), not runtime-growing, so no
/// LRU/eviction is needed here (unlike a string-keyed cache, which would
/// have to evict).
static STATE: RwLock<Option<HashMap<Identifier, CallsiteState>>> = RwLock::new(None);

fn with_state<R>(f: impl FnOnce(&mut HashMap<Identifier, CallsiteState>) -> R) -> R {
    let mut guard = STATE.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    f(map)
}

/// A [`tracing_subscriber::layer::Filter`] that suppresses a callsite's
/// events past `threshold` within `window`. Applied to `console_layer` only
/// (`init_tracing()`) -- the file log's own `log-level` knob (`LOG-003`)
/// already governs its verbosity independently.
pub struct RateLimitFilter {
    threshold: u32,
    window: Duration,
}

impl RateLimitFilter {
    pub fn new(threshold: u32, window: Duration) -> Self {
        Self { threshold, window }
    }
}

impl<S> Filter<S> for RateLimitFilter {
    fn enabled(&self, meta: &Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        let id = meta.callsite();
        let now = Instant::now();
        with_state(|map| {
            let entry = map.entry(id).or_insert_with(|| CallsiteState {
                window_start: now,
                count: 0,
                suppressed_since_shown: 0,
            });
            let (allow, new_window_start, new_count) = ratelimit_decision(
                now,
                entry.window_start,
                entry.count,
                self.threshold,
                self.window,
            );
            entry.window_start = new_window_start;
            entry.count = new_count;
            if allow {
                if entry.suppressed_since_shown > 0 {
                    // Deliberately bypasses the `tracing` dispatcher (a
                    // plain `eprintln!`, same technique the panic hook uses
                    // for its own out-of-band write) rather than emitting a
                    // synthetic `tracing::event!` from inside a `Filter`
                    // callback: `tracing`'s reentrancy guard treats a
                    // dispatch made from within a currently-dispatching
                    // callback as nested and silently no-ops it, so a
                    // tracing-based summary line would never actually
                    // reach the console. Console output is line-buffered
                    // stdout either way (`tracing_subscriber::fmt::layer()`
                    // default), so this interleaves safely with it.
                    eprintln!(
                        "log_ratelimit: {} event(s) suppressed for {}:{} (target {}) in the last window",
                        entry.suppressed_since_shown,
                        meta.file().unwrap_or("?"),
                        meta.line().unwrap_or(0),
                        meta.target(),
                    );
                }
                entry.suppressed_since_shown = 0;
            } else {
                entry.suppressed_since_shown += 1;
            }
            allow
        })
    }
}

#[cfg(test)]
mod ratelimit_decision_tests {
    use super::*;

    #[test]
    fn first_event_in_fresh_window_is_allowed() {
        let now = Instant::now();
        let (allow, _, count) = ratelimit_decision(now, now, 0, 5, Duration::from_secs(60));
        assert!(allow);
        assert_eq!(count, 1);
    }

    #[test]
    fn stays_allowed_up_to_and_including_threshold() {
        let window_start = Instant::now();
        let window = Duration::from_secs(60);
        let now = window_start + Duration::from_secs(1);
        let (allow, _, count) = ratelimit_decision(now, window_start, 4, 5, window);
        assert!(allow);
        assert_eq!(count, 5);
    }

    #[test]
    fn exceeds_threshold_is_suppressed() {
        let window_start = Instant::now();
        let window = Duration::from_secs(60);
        let now = window_start + Duration::from_secs(1);
        let (allow, _, count) = ratelimit_decision(now, window_start, 5, 5, window);
        assert!(!allow);
        assert_eq!(count, 6);
    }

    #[test]
    fn window_elapsed_resets_and_allows_regardless_of_prior_count() {
        let window_start = Instant::now();
        let window = Duration::from_secs(60);
        let now = window_start + Duration::from_secs(61);
        let (allow, new_window_start, count) =
            ratelimit_decision(now, window_start, 500, 5, window);
        assert!(allow);
        assert_eq!(count, 1);
        assert_eq!(new_window_start, now);
    }

    #[test]
    fn window_boundary_is_inclusive_of_reset() {
        let window_start = Instant::now();
        let window = Duration::from_secs(60);
        let now = window_start + window;
        let (allow, new_window_start, count) = ratelimit_decision(now, window_start, 10, 5, window);
        assert!(allow);
        assert_eq!(count, 1);
        assert_eq!(new_window_start, now);
    }
}
