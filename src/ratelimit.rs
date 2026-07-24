//! Rate limiting for inbound control streams (HARDEN-002/004/005).
//!
//! Control-plane messages (`MemberSync`/`BlobUpdated` triggers, `MeshHello`,
//! invite gossip) are cheap to send but can be expensive to process — a single
//! `MemberSync` drives a pkarr resolve and, on a hash change, a blob fetch + DNS
//! rebuild. They carry no per-message authentication,
//! so any peer sharing a network can spam them. [`ControlGate`] guards each
//! control-listener task with a token bucket (the `ratelimit` crate) plus a
//! strike counter: over-budget messages are dropped, and a peer that sustains a
//! flood eventually trips [`Verdict::Close`] so the caller can drop the
//! connection. A peer that only bursts briefly is never penalized — strikes
//! decay on every admitted message.
//!
//! One [`ControlGate`] lives per listener task (each task owns exactly one
//! peer's connection). [`GlobalRateLimiter`] is the one exception: a single
//! daemon-wide instance, shared (via `Arc`) across every connection's control
//! listener, bounding the aggregate control-plane workload rather than any one
//! connection. `ControlGate::check_with_global` consults both — a message is
//! processed only if neither gate is over budget.
//!
//! Both the per-connection and global policies are configurable
//! (`config::RateLimitConfig`, HARDEN-005): `None` on any field falls back to
//! the compiled default below.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use ratelimit::Ratelimiter;

use crate::config;

/// Burst of control messages absorbed instantly before throttling kicks in.
const CAPACITY: u64 = 5;
/// Sustained refill rate, in tokens per second.
const REFILL_PER_SEC: u64 = 1;
/// Net over-budget messages (drops minus admits) before the connection is closed.
const STRIKE_LIMIT: u32 = 20;

/// Burst absorbed instantly by the daemon-wide shared bucket.
const GLOBAL_CAPACITY: u64 = 10;
/// Sustained refill rate of the shared bucket, in tokens per second.
const GLOBAL_REFILL_PER_SEC: u64 = 3;
/// Net over-budget messages before the shared bucket starts requesting closes.
const GLOBAL_STRIKE_LIMIT: u32 = 50;

/// What to do with one inbound control message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A token was available — dispatch the message normally.
    Allow,
    /// Over budget — drop the message; the connection is still healthy.
    Drop,
    /// Sustained flood — drop the message and close the connection.
    Close,
}

/// Token-bucket guard over one connection's inbound control messages.
pub struct ControlGate {
    limiter: Ratelimiter,
    strikes: u32,
    strike_limit: u32,
}

impl ControlGate {
    /// Build a gate using the configured policy (`config::RateLimitConfig`),
    /// falling back to the compiled default on any unset field or unreadable
    /// config. Reads config fresh on every call — cheap, since this runs once
    /// per connection, not on the per-message hot path.
    pub fn new() -> Self {
        let overrides = config::load().map(|c| c.ratelimit).unwrap_or_default();
        Self::with_params(
            overrides.capacity.unwrap_or(CAPACITY),
            overrides.refill_per_sec.unwrap_or(REFILL_PER_SEC),
            overrides.strike_limit.unwrap_or(STRIKE_LIMIT),
        )
    }

    /// Build a gate with explicit parameters (used by tests).
    pub fn with_params(capacity: u64, refill_per_sec: u64, strike_limit: u32) -> Self {
        let limiter = Ratelimiter::builder(refill_per_sec, Duration::from_secs(1))
            .max_tokens(capacity)
            .initial_available(capacity)
            .build()
            .expect("valid ratelimiter parameters");
        Self {
            limiter,
            strikes: 0,
            strike_limit,
        }
    }

    /// Account for one inbound control message and decide what to do with it.
    pub fn check(&mut self) -> Verdict {
        match self.limiter.try_wait() {
            Ok(()) => {
                self.strikes = self.strikes.saturating_sub(1);
                Verdict::Allow
            }
            Err(_) => {
                self.strikes = self.strikes.saturating_add(1);
                if self.strikes >= self.strike_limit {
                    Verdict::Close
                } else {
                    Verdict::Drop
                }
            }
        }
    }

    /// Check this connection's own gate, then (only if it Allows) the shared
    /// daemon-wide gate too (HARDEN-004) — both must Allow for the message to
    /// be processed. A per-connection Drop/Close short-circuits: the message
    /// is already being dropped, so there is no reason to also spend a global
    /// token on it. A `Close` from the global gate closes this connection
    /// even though it individually behaved — there is no single "the"
    /// abusive connection to target once the aggregate budget is blown, so
    /// this is a pragmatic, not an accusatory, choice.
    pub fn check_with_global(&mut self, global: &GlobalRateLimiter) -> Verdict {
        match self.check() {
            Verdict::Allow => global.check(),
            other => other,
        }
    }
}

impl Default for ControlGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Daemon-wide token bucket shared across every connection's [`ControlGate`]
/// (HARDEN-004). Built once at daemon bootstrap and cloned (via `Arc`) into
/// every network's [`crate::daemon::MeshCtx`].
///
/// Uses the `ratelimit` crate's own lock-free, atomics-based `Ratelimiter`
/// (`try_wait` takes `&self`) plus a bare `AtomicU32` strike counter, so
/// `check` takes `&self` — no `Mutex` needed for concurrent access from many
/// connection tasks at once.
pub struct GlobalRateLimiter {
    limiter: Ratelimiter,
    strikes: AtomicU32,
    strike_limit: u32,
}

impl GlobalRateLimiter {
    /// Build the shared gate from the configured policy, falling back to the
    /// compiled default on any unset field. Called once at daemon bootstrap.
    pub fn from_config(overrides: &config::RateLimitConfig) -> Self {
        Self::with_params(
            overrides.global_capacity.unwrap_or(GLOBAL_CAPACITY),
            overrides.global_refill_per_sec.unwrap_or(GLOBAL_REFILL_PER_SEC),
            overrides.global_strike_limit.unwrap_or(GLOBAL_STRIKE_LIMIT),
        )
    }

    /// Build a gate with explicit parameters (used by tests).
    pub fn with_params(capacity: u64, refill_per_sec: u64, strike_limit: u32) -> Self {
        let limiter = Ratelimiter::builder(refill_per_sec, Duration::from_secs(1))
            .max_tokens(capacity)
            .initial_available(capacity)
            .build()
            .expect("valid ratelimiter parameters");
        Self {
            limiter,
            strikes: AtomicU32::new(0),
            strike_limit,
        }
    }

    /// Account for one inbound control message against the shared budget.
    pub fn check(&self) -> Verdict {
        match self.limiter.try_wait() {
            Ok(()) => {
                // Best-effort decay: a lost race just leaves strikes one
                // higher than ideal for a moment, corrected by the next Allow.
                let _ = self
                    .strikes
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |s| {
                        Some(s.saturating_sub(1))
                    });
                Verdict::Allow
            }
            Err(_) => {
                let prev = self.strikes.fetch_add(1, Ordering::Relaxed);
                if prev.saturating_add(1) >= self.strike_limit {
                    Verdict::Close
                } else {
                    Verdict::Drop
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_a_burst_up_to_capacity() {
        let mut gate = ControlGate::with_params(5, 1, 100);
        // The initial bucket holds `capacity` tokens, all admitted instantly.
        for _ in 0..5 {
            assert_eq!(gate.check(), Verdict::Allow);
        }
        // The next message has no token left this instant — dropped, not closed.
        assert_eq!(gate.check(), Verdict::Drop);
    }

    #[test]
    fn sustained_flood_trips_close() {
        let mut gate = ControlGate::with_params(3, 1, 10);
        // Drain the bucket.
        for _ in 0..3 {
            assert_eq!(gate.check(), Verdict::Allow);
        }
        // Keep hammering with no refill: strikes climb to the limit, then Close.
        let mut verdicts = Vec::new();
        for _ in 0..20 {
            verdicts.push(gate.check());
        }
        assert!(
            verdicts.contains(&Verdict::Close),
            "expected a Close verdict under sustained flood"
        );
    }

    #[test]
    fn strikes_decay_so_a_chatty_peer_is_never_closed() {
        // A run of admits drives strikes back down to zero, so an earlier short
        // burst of drops can never accumulate into a Close.
        let mut gate = ControlGate::with_params(2, 1, 5);
        gate.strikes = 4; // simulate a prior near-miss burst
        // Two admitted messages (capacity 2) decay strikes by two.
        assert_eq!(gate.check(), Verdict::Allow);
        assert_eq!(gate.check(), Verdict::Allow);
        assert_eq!(gate.strikes, 2);
        // The next over-budget message is a Drop (strikes 3 < limit 5), not Close.
        assert_eq!(gate.check(), Verdict::Drop);
    }

    #[test]
    fn global_gate_admits_a_burst_up_to_capacity() {
        let global = GlobalRateLimiter::with_params(4, 1, 100);
        for _ in 0..4 {
            assert_eq!(global.check(), Verdict::Allow);
        }
        assert_eq!(global.check(), Verdict::Drop);
    }

    #[test]
    fn global_gate_sustained_flood_trips_close() {
        let global = GlobalRateLimiter::with_params(2, 1, 5);
        for _ in 0..2 {
            assert_eq!(global.check(), Verdict::Allow);
        }
        let mut verdicts = Vec::new();
        for _ in 0..10 {
            verdicts.push(global.check());
        }
        assert!(
            verdicts.contains(&Verdict::Close),
            "expected a Close verdict under sustained flood"
        );
    }

    #[test]
    fn check_with_global_drops_without_consulting_global_when_local_drops() {
        // Drain the local gate's one token, then exercise the combined check
        // against a fresh global gate with plenty of capacity. The combined
        // check must still Drop (local governs first) and must not have
        // spent a global token doing so.
        let mut local = ControlGate::with_params(1, 1, 100);
        assert_eq!(local.check(), Verdict::Allow);
        let global = GlobalRateLimiter::with_params(1, 1, 100);
        assert_eq!(local.check_with_global(&global), Verdict::Drop);
        // Global token still available — proves it was never consulted.
        assert_eq!(global.check(), Verdict::Allow);
    }

    #[test]
    fn check_with_global_requires_both_gates_to_allow() {
        let mut local = ControlGate::with_params(5, 1, 100);
        let global = GlobalRateLimiter::with_params(1, 1, 100);
        // First message: both gates have capacity.
        assert_eq!(local.check_with_global(&global), Verdict::Allow);
        // Second message: local still has tokens, but global is now empty.
        assert_eq!(local.check_with_global(&global), Verdict::Drop);
    }
}
