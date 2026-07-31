//! Packet and byte counters for the forwarding data path (iroh-metrics counters).
//!
//! Replaces hand-rolled atomics with `iroh_metrics::Counter` and labeled drop
//! counters via `Family<DropLabels, Counter>`. The counters feed `tetron status
//! --json`'s live traffic/drops/fragmentation display (STATUS-002/MTU-DIAG-001).
//! No periodic logging — only a session-summary line on shutdown (LOG-001).

use std::sync::Arc;
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
            stats.drop_count(DropReason::Malformed)
                + stats.drop_count(DropReason::NoPeer),
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
