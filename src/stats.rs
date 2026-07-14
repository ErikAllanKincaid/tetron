//! Packet and byte counters for the forwarding data path (iroh-metrics counters).
//!
//! Replaces hand-rolled atomics with `iroh_metrics::Counter` and labeled drop
//! counters via `Family<DropLabels, Counter>`. A background logger prints
//! 30-second interval deltas and a session summary on shutdown.

use std::sync::Arc;
use std::time::{Duration, Instant};

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
}

impl DropReason {
    const ALL: [DropReason; 5] = [
        DropReason::SendFailure,
        DropReason::NoPeer,
        DropReason::Malformed,
        DropReason::Backpressure,
        DropReason::Spoof,
    ];
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

    fn drop_count(&self, reason: DropReason) -> u64 {
        self.drops
            .get(&DropLabels { reason })
            .map(|c| c.get())
            .unwrap_or(0)
    }

    fn total_drops(&self) -> u64 {
        DropReason::ALL.iter().map(|r| self.drop_count(*r)).sum()
    }


    pub fn spawn_logger(self: &Arc<Self>, token: CancellationToken) {
        let stats = self.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let mut prev_rx = 0u64;
            let mut prev_tx = 0u64;
            let mut prev_bytes_rx = 0u64;
            let mut prev_bytes_tx = 0u64;
            let mut prev_drops = 0u64;

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {
                        let rx = stats.packets_rx.get();
                        let tx = stats.packets_tx.get();
                        let brx = stats.bytes_rx.get();
                        let btx = stats.bytes_tx.get();
                        let drops = stats.total_drops();

                        tracing::info!(
                            rx = rx - prev_rx,
                            tx = tx - prev_tx,
                            bytes_rx = brx - prev_bytes_rx,
                            bytes_tx = btx - prev_bytes_tx,
                            drops = drops - prev_drops,
                            "(30s)"
                        );

                        prev_rx = rx;
                        prev_tx = tx;
                        prev_bytes_rx = brx;
                        prev_bytes_tx = btx;
                        prev_drops = drops;
                    }
                    _ = token.cancelled() => {
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
                        return;
                    }
                }
            }
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
        assert_eq!(stats.total_drops(), 3);
    }

}
