//! Packet and byte counters for the forwarding data path (iroh-metrics counters).
//!
//! Replaces hand-rolled atomics with `iroh_metrics::Counter` and labeled drop
//! counters via `Family<DropLabels, Counter>`. The counters feed `tetron status
//! --json`'s live traffic/drops/fragmentation display (STATUS-002/MTU-DIAG-001).
//! No periodic logging — only a session-summary line on shutdown (LOG-001).
//!
//! An optional proactive drop-rate monitor (`DropMonitor`, LOG-002) can be
//! initialised at daemon start via [`init_drop_monitor`]. When active, every
//! `ForwardMetrics::record_drop` call also increments a per-reason atomic
//! bucket; a background task warns when a bucket's count exceeds the configured
//! threshold within a window.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use iroh_metrics::{Counter, EncodeLabelSet, EncodeLabelValue, Family, MetricsGroup};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, EncodeLabelValue)]
pub enum DropReason {
    SendFailure,
    NoPeer,
    Malformed,
    /// Outbound packet dropped at the application boundary because the peer's
    /// QUIC datagram send buffer was too full to accept it without evicting an
    /// already-queued (older) packet. Dropping the *new* packet here (drop-newest)
    /// is preferable to letting QUIC drop the *oldest* queued one — for a VPN the
    /// oldest queued packet is more likely to be useful (already-accepted work)
    /// than a fresh one arriving into a saturated link.
    Backpressure,
    /// Inbound datagram whose source IP did not match the sending peer's
    /// assigned mesh address (ingress anti-spoofing). A peer may only inject
    /// packets sourced from its own mesh IP.
    Spoof,
    /// An oversized outbound packet could not be fragmented at all (MTU-DIAG-001):
    /// IPv4's checksum/options guard rejected it, or the IPv6 envelope
    /// header didn't fit under the connection's `max_datagram_size`.
    /// Previously indistinguishable from a generic `SendFailure` -- this is
    /// the exact signal that would have surfaced the FRAG-001/F-04
    /// live regression without needing to grep raw logs.
    FragmentationFailed,
}

impl DropReason {}

/// Compiled defaults for [`DropMonitor`] (LOG-002).
pub const DROP_MONITOR_WINDOW_SECS: u64 = 60;
pub const DROP_MONITOR_THRESHOLD: u64 = 0; // 0 = disabled
pub const DROP_MONITOR_COOLDOWN_SECS: u64 = 300;

/// Per-reason index into the monitor's fixed-size arrays. Must match
/// `DropReason` variant order.
fn reason_index(r: DropReason) -> usize {
    match r {
        DropReason::SendFailure => 0,
        DropReason::NoPeer => 1,
        DropReason::Malformed => 2,
        DropReason::Backpressure => 3,
        DropReason::Spoof => 4,
        DropReason::FragmentationFailed => 5,
    }
}

const fn reason_name(idx: usize) -> &'static str {
    match idx {
        0 => "SendFailure",
        1 => "NoPeer",
        2 => "Malformed",
        3 => "Backpressure",
        4 => "Spoof",
        5 => "FragmentationFailed",
        _ => "Unknown",
    }
}

const DROP_REASON_COUNT: usize = 6;

/// Proactive drop-rate monitor (LOG-002). A background task reads and resets
/// per-reason atomic buckets every `window_secs`; if any bucket's count meets
/// or exceeds `threshold` and the per-reason cooldown has elapsed, a single
/// `warn!` is emitted.
///
/// Dropped into a global [`OnceLock`] at daemon start so
/// [`ForwardMetrics::record_drop`] can increment both the metrics counter and
/// the monitor bucket with zero call-site changes.
pub struct DropMonitor {
    buckets: [AtomicU64; DROP_REASON_COUNT],
    last_warned: [AtomicU64; DROP_REASON_COUNT],
    window_secs: u64,
    threshold: u64,
    cooldown_secs: u64,
}

impl DropMonitor {
    pub fn new(window_secs: u64, threshold: u64, cooldown_secs: u64) -> Self {
        Self {
            buckets: Default::default(),
            last_warned: Default::default(),
            window_secs,
            threshold,
            cooldown_secs,
        }
    }

    /// Increment the per-reason bucket for a drop event. Called from
    /// [`ForwardMetrics::record_drop`] when the global monitor is set.
    #[inline]
    pub fn record_drop(&self, reason: DropReason) {
        self.buckets[reason_index(reason)].fetch_add(1, Ordering::Relaxed);
    }

    /// Spawn the background monitoring task. Runs every `window_secs`,
    /// checking each bucket against the threshold and emitting a `warn!` if
    /// exceeded and the cooldown has elapsed. Exits when `token` is cancelled.
    pub fn spawn_monitor(self: &Arc<Self>, token: CancellationToken) {
        let monitor = self.clone();
        let interval = std::time::Duration::from_secs(self.window_secs);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        monitor.check_and_warn();
                    }
                    _ = token.cancelled() => return,
                }
            }
        });
    }

    fn check_and_warn(&self) {
        if self.threshold == 0 {
            return; // disabled
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for i in 0..DROP_REASON_COUNT {
            let count = self.buckets[i].swap(0, Ordering::Relaxed);
            if count < self.threshold {
                continue;
            }
            let last = self.last_warned[i].load(Ordering::Relaxed);
            if now.saturating_sub(last) < self.cooldown_secs {
                continue;
            }
            self.last_warned[i].store(now, Ordering::Relaxed);
            let rate = count as f64 / self.window_secs as f64;
            tracing::warn!(
                reason = reason_name(i),
                count,
                window_secs = self.window_secs,
                rate,
                "drop rate exceeded threshold"
            );
        }
    }
}

/// Global drop monitor, installed once at daemon start. Read by
/// [`ForwardMetrics::record_drop`] with zero call-site overhead for the
/// disabled (default) case — a single atomic load of a null pointer.
static DROP_MONITOR: std::sync::OnceLock<Arc<DropMonitor>> = std::sync::OnceLock::new();

/// Initialise the global drop monitor from config overrides. Must be called
/// before any packet flow starts (typically from [`run_daemon`]) and only once.
/// Uses compiled defaults for any `None` field.
pub fn init_drop_monitor(config: &crate::config::DropMonitorConfig, token: CancellationToken) {
    let window = config.window_secs.unwrap_or(DROP_MONITOR_WINDOW_SECS);
    let threshold = config.threshold.unwrap_or(DROP_MONITOR_THRESHOLD);
    let cooldown = config.cooldown_secs.unwrap_or(DROP_MONITOR_COOLDOWN_SECS);
    let monitor = Arc::new(DropMonitor::new(window, threshold, cooldown));
    if threshold > 0 {
        monitor.spawn_monitor(token);
        tracing::info!(
            window_secs = window,
            threshold,
            cooldown_secs = cooldown,
            "drop monitor active"
        );
    }
    let _ = DROP_MONITOR.set(monitor);
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, EncodeLabelSet)]
pub struct DropLabels {
    pub reason: DropReason,
}

#[derive(Debug, MetricsGroup)]
#[metrics(name = "tetron", default)]
pub struct ForwardMetrics {
    /// Total packets received from peers
    pub packets_rx: Counter,
    /// Total packets sent to peers
    pub packets_tx: Counter,
    /// Total bytes received from peers
    pub bytes_rx: Counter,
    /// Total bytes sent to peers
    pub bytes_tx: Counter,
    /// Dropped packets by reason
    pub drops: Family<DropLabels, Counter>,
    /// Original oversized IPv4 packets that were successfully split
    /// (MTU-DIAG-001) -- incremented once per packet, not once per wire
    /// fragment.
    pub fragmented_ipv4: Counter,
    /// Original oversized IPv6 packets that were successfully split into the
    /// tetron-internal envelope (MTU-DIAG-001) -- incremented once per
    /// packet, not once per wire fragment.
    pub fragmented_ipv6: Counter,
}

impl ForwardMetrics {
    pub fn record_rx(&self, bytes: usize) {
        self.packets_rx.inc();
        self.bytes_rx.inc_by(bytes as u64);
    }

    pub fn record_tx(&self, bytes: usize) {
        self.packets_tx.inc();
        self.bytes_tx.inc_by(bytes as u64);
    }

    pub fn record_drop(&self, reason: DropReason) {
        self.drops.get_or_create(&DropLabels { reason }).inc();
        if let Some(monitor) = DROP_MONITOR.get() {
            monitor.record_drop(reason);
        }
    }

    pub fn record_fragmented_ipv4(&self) {
        self.fragmented_ipv4.inc();
    }

    pub fn record_fragmented_ipv6(&self) {
        self.fragmented_ipv6.inc();
    }

    pub(crate) fn drop_count(&self, reason: DropReason) -> u64 {
        self.drops
            .get(&DropLabels { reason })
            .map(|c| c.get())
            .unwrap_or(0)
    }

    pub fn spawn_logger(self: &Arc<Self>, token: CancellationToken) {
        let stats = self.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            token.cancelled().await;
            let duration = start.elapsed();
            let mins = duration.as_secs() / 60;
            let secs = duration.as_secs() % 60;
            let total_bytes = stats.bytes_rx.get() + stats.bytes_tx.get();

            tracing::info!(
                duration = format!("{}m{}s", mins, secs),
                total_rx = stats.packets_rx.get(),
                total_tx = stats.packets_tx.get(),
                total_bytes,
                "session complete"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_rx() {
        let stats = ForwardMetrics::default();
        stats.record_rx(100);
        stats.record_rx(200);
        assert_eq!(stats.packets_rx.get(), 2);
        assert_eq!(stats.bytes_rx.get(), 300);
    }

    #[test]
    fn test_record_tx() {
        let stats = ForwardMetrics::default();
        stats.record_tx(500);
        assert_eq!(stats.packets_tx.get(), 1);
        assert_eq!(stats.bytes_tx.get(), 500);
    }

    #[test]
    fn test_record_drop() {
        let stats = ForwardMetrics::default();
        stats.record_drop(DropReason::Malformed);
        stats.record_drop(DropReason::NoPeer);
        stats.record_drop(DropReason::Malformed);
        assert_eq!(
            stats
                .drops
                .get(&DropLabels {
                    reason: DropReason::Malformed
                })
                .unwrap()
                .get(),
            2
        );
        assert_eq!(
            stats
                .drops
                .get(&DropLabels {
                    reason: DropReason::NoPeer
                })
                .unwrap()
                .get(),
            1
        );
        assert_eq!(
            stats.drop_count(DropReason::Malformed) + stats.drop_count(DropReason::NoPeer),
            3
        );
    }

    #[test]
    fn test_fragmentation_failed_is_distinct_from_send_failure() {
        let stats = ForwardMetrics::default();
        stats.record_drop(DropReason::FragmentationFailed);
        stats.record_drop(DropReason::SendFailure);
        assert_eq!(stats.drop_count(DropReason::FragmentationFailed), 1);
        assert_eq!(stats.drop_count(DropReason::SendFailure), 1);
        assert_eq!(
            stats.drop_count(DropReason::FragmentationFailed)
                + stats.drop_count(DropReason::SendFailure),
            2
        );
    }

    #[test]
    fn test_record_fragmented_counters() {
        let stats = ForwardMetrics::default();
        stats.record_fragmented_ipv4();
        stats.record_fragmented_ipv4();
        stats.record_fragmented_ipv6();
        assert_eq!(stats.fragmented_ipv4.get(), 2);
        assert_eq!(stats.fragmented_ipv6.get(), 1);
    }
}
