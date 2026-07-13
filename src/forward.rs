//! Mesh packet forwarding between TUN device and peer QUIC connections.
//!
//! Three concurrent tasks handle the data plane:
//! - [`run_mesh`]: reads outgoing packets from TUN, routes to correct peer via [`PeerTable`]
//! - [`spawn_peer_reader`]: one per peer, reads incoming datagrams and forwards to TUN writer
//! - [`spawn_tun_writer`]: single task, writes incoming packets to the TUN device

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use bytes::{Bytes, BytesMut};
use iroh::EndpointId;
use iroh::endpoint::{Connection, ConnectionError, VarInt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::packet;
use crate::peers::PeerTable;
use crate::stats::{DropReason, ForwardMetrics};

/// Maximum datagram size accepted from a peer. Anything larger is dropped before
/// being parsed or written to the TUN device, bounding memory use under a flood
/// of oversized datagrams from a malicious or buggy peer.
const MAX_PEER_DATAGRAM: usize = 1500;

/// Size of the TUN read pool. One allocation is amortized across the ~50
/// datagrams that fit in a chunk: each packet is sliced off with a zero-copy
/// `split_to(n).freeze()`, and a fresh chunk is only allocated once the current
/// one is exhausted (the old chunk stays alive via the `Bytes` already handed to
/// quinn and is freed as those datagrams are sent).
const TX_POOL_CHUNK: usize = 64 * 1024;

/// Decision returned by [`evaluate_inbound`] for a datagram received from a peer.
pub(crate) enum InboundDecision {
    /// Packet passed validation and may be written to the TUN.
    Accept,
    /// Dropped: too large or not a parseable IP packet.
    DropMalformed,
    /// Dropped: the packet's source IP is not the sending peer's assigned mesh
    /// address. A peer may only source packets from its own mesh IP, so this
    /// blocks one peer from impersonating another's IP (ingress anti-spoofing).
    DropSpoof,
}

/// Pure evaluation of an inbound peer datagram against basic packet validity and
/// the ingress anti-spoof check. Extracted from [`spawn_peer_reader`] so it can
/// be unit-tested.
///
/// Non-IP / truncated / oversized packets are rejected (`DropMalformed`) rather
/// than passed through. Packet *filtering* is the host firewall's job
/// (nftables/ufw on the TUN interface); the mesh no longer runs a userspace
/// firewall (MINIMAL-010).
pub(crate) fn evaluate_inbound(
    datagram: &[u8],
    peer_ip: Ipv4Addr,
    peer_ipv6: Ipv6Addr,
) -> InboundDecision {
    if datagram.len() > MAX_PEER_DATAGRAM {
        return InboundDecision::DropMalformed;
    }
    let Some(info) = packet::parse_packet_info(datagram) else {
        return InboundDecision::DropMalformed;
    };
    // Ingress anti-spoofing: a peer may only inject packets sourced from its own
    // assigned mesh address. Anything else (e.g. one peer forging another's mesh
    // IP) is dropped before any in-daemon listener sees it, so
    // identity-from-source-IP stays trustworthy.
    let src_ok = match info.src_ip {
        IpAddr::V4(v4) => v4 == peer_ip,
        IpAddr::V6(v6) => v6 == peer_ipv6,
    };
    if !src_ok {
        return InboundDecision::DropSpoof;
    }
    InboundDecision::Accept
}

/// Application close code a peer sends when it deliberately leaves a network
/// (`torpedo leave`). Distinguishes an intentional departure from a transient drop
/// (timeout/reset), so only deliberate leaves prune the canonical member list.
pub const LEAVE_CODE: u32 = 0x1ea5e;

/// Application close code used to drop a peer that floods the control plane with
/// messages (see [`crate::ratelimit::ControlGate`]). Distinct from
/// [`LEAVE_CODE`]: a flooded-out peer did not depart the network, so it is
/// treated as a non-intentional disconnect (the peer may reconnect; no quarantine).
pub const ABUSE_CODE: u32 = 0xab05e;

/// Application close code a coordinator (or any member pruning a stale roster
/// entry) sends when it removes a peer from the network (`torpedo kick`). On the
/// receiving (kicked) side it is treated like [`LEAVE_CODE`] — an intentional
/// disconnect — so the kicked node stops reconnecting instead of churning back
/// into the coordinator's pending queue. The pruning side does not observe its
/// own close code (that read is a local close), so it relies on the shared
/// `pruned_peers` set to suppress its reconnect loop.
pub const KICK_CODE: u32 = 0x14ced;

/// Sent by [`spawn_peer_reader`] when a peer connection drops,
/// consumed by the reconnect loop (joiner) or cleanup task (coordinator).
pub struct DisconnectEvent {
    pub endpoint_id: EndpointId,
    pub ip: Ipv4Addr,
    pub ipv6: Ipv6Addr,
    /// The network whose connection dropped. A multi-homed peer keeps its routes
    /// in the other networks; only this network's connection is torn down.
    pub network: String,
    /// True when the peer closed gracefully with [`LEAVE_CODE`] (it ran
    /// `torpedo leave`), as opposed to a timeout/reset.
    pub intentional: bool,
    /// [`Connection::stable_id`] of the connection that dropped, so a consumer
    /// can tell whether the connection currently stored for this peer is still
    /// the one that died. `None` for a synthetic kick that is not tied to a live
    /// connection (the cold-restore reconnect seed), which always proceeds.
    ///
    /// Guards an ABA race: when a peer's process is killed and it re-dials with
    /// the same identity, the coordinator registers the fresh connection before
    /// the old one's idle timeout fires. Without this id, the stale connection's
    /// delayed disconnect would evict the fresh connection and drop the peer.
    pub conn_stable_id: Option<usize>,
}

/// Shared data-plane handles threaded into every per-peer reader. All fields are
/// cheap `Clone` (channels and Arc-backed handles), so a reader is spawned with a
/// single bundle instead of six separate arguments. Built per spawn from the
/// daemon's `MeshCtx` via `MeshCtx::forward_ctx`.
pub struct ForwardCtx {
    /// Swappable sender cell for the TUN writer. Peer readers outlive TUN
    /// attach/detach cycles (the control plane stays up across a VPN toggle), so
    /// they resolve the current writer per packet via `tun_tx.load_full()` rather
    /// than capturing one sender. After a detach + re-attach the cell points at
    /// the new writer, so a reader spawned during the first `up()` keeps
    /// forwarding after the next one. See [`DaemonState::attach_tun`].
    pub tun_tx: Arc<arc_swap::ArcSwap<mpsc::Sender<Bytes>>>,
    pub disconnect_tx: mpsc::Sender<DisconnectEvent>,
    pub token: CancellationToken,
    pub stats: Arc<ForwardMetrics>,
}

/// True when a parsed packet is a DNS query addressed to the magic resolver IP.
pub(crate) fn is_magic_dns(info: &packet::PacketInfo) -> bool {
    info.dst_port == 53 && info.dst_ip == IpAddr::V4(crate::dns::magic_dns_v4_node())
}

/// Main TUN read loop. Reads packets from the TUN device, extracts the destination IP,
/// looks up the peer in [`PeerTable`], and sends the packet as a QUIC datagram.
/// Packets with no matching peer are silently dropped.
#[allow(clippy::too_many_arguments)]
pub async fn run_mesh<R: crate::tun::TunRead>(
    mut tun: R,
    peers: PeerTable,
    token: CancellationToken,
    stats: Arc<ForwardMetrics>,
    resolver: Arc<crate::dns_resolver::Resolver>,
    tun_tx: mpsc::Sender<Bytes>,
) -> Result<()> {
    let mut pool = BytesMut::with_capacity(TX_POOL_CHUNK);
    loop {
        // Ensure a full MTU of contiguous spare capacity before reading (a short
        // buffer would truncate the packet). `reserve` reuses the current chunk
        // until it's exhausted, then allocates a fresh one — so allocation is
        // amortized across many packets instead of paid per packet.
        if pool.capacity() < MAX_PEER_DATAGRAM {
            pool.reserve(TX_POOL_CHUNK);
        }
        // Race the read against cancellation, but return only the byte count so
        // no borrow of `pool` escapes the `select!` (it's reused right below).
        let n = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = tun.read_into(&mut pool) => result?,
        };
        if n == 0 {
            continue;
        }
        // Zero-copy hand-off: slice the packet out of the pool as an owned
        // `Bytes` sharing the chunk's allocation — no copy, no per-packet malloc.
        let pkt = pool.split_to(n).freeze();
        tracing::debug!(len = n, first_byte = pkt[0], "TUN read");
        let Some(info) = packet::parse_packet_info(&pkt) else {
            tracing::debug!(len = n, "not IP, dropping");
            continue;
        };
        if is_magic_dns(&info) {
            let resolver = resolver.clone();
            let tun_tx = tun_tx.clone();
            let pkt = pkt.clone();
            tokio::spawn(async move {
                resolver.handle_tun_query(&pkt, &info, &tun_tx).await;
            });
            continue; // do not fall through to peer routing
        }
        let lookup = match info.dst_ip {
            IpAddr::V4(v4) => peers.lookup_v4(&v4),
            IpAddr::V6(v6) => peers.lookup_v6(&v6),
        };
        let Some(route) = lookup else {
            tracing::debug!(dst = %info.dst_ip, "no peer for dst");
            stats.record_drop(DropReason::NoPeer);
            continue;
        };
        // Reachability is "we share a network" — enforced by connection
        // existence. Packet filtering is the host firewall's job.
        tracing::debug!(dst = %info.dst_ip, "routing to peer");
        // Drop-newest at the application boundary: if the peer's QUIC datagram send
        // buffer is too full to accept this packet without evicting an already-queued
        // (older) one, drop the *new* packet here instead of calling `send_datagram`,
        // which would drop the *oldest* queued packet (see N6 in the datagram audit).
        // This keeps the send path non-blocking (no cross-peer head-of-line blocking
        // in this single TUN read loop) while preferring drop-newest over drop-oldest.
        // Full per-peer backpressure (`send_datagram_wait` in a per-peer writer task)
        // is the sized follow-up that needs the e2e harness to land safely.
        if route.conn.datagram_send_buffer_space() < n {
            tracing::trace!(
                dst = %info.dst_ip,
                space = route.conn.datagram_send_buffer_space(),
                len = n,
                "datagram send buffer full; dropping newest",
            );
            stats.record_drop(DropReason::Backpressure);
            continue;
        }
        match route.conn.send_datagram(pkt) {
            Ok(()) => stats.record_tx(n),
            Err(e) => {
                tracing::debug!(dst = %info.dst_ip, error = %e, "datagram send failed");
                stats.record_drop(DropReason::SendFailure);
            }
        }
    }
}

/// Spawns a task that reads QUIC datagrams from a single peer connection and
/// forwards them to the TUN writer via `tun_tx`. On connection loss, sends a
/// [`DisconnectEvent`] and exits.
pub fn spawn_peer_reader(
    conn: Connection,
    peer_id: EndpointId,
    peer_ip: Ipv4Addr,
    peer_ipv6: Ipv6Addr,
    network: String,
    ctx: ForwardCtx,
) -> JoinHandle<()> {
    let ForwardCtx {
        tun_tx,
        disconnect_tx,
        token,
        stats,
    } = ctx;
    use tracing::Instrument as _;
    // Tag every event from this reader (drops, connection-lost) with the peer
    // and network so the report bundle's logs are correlatable per peer.
    let span = tracing::info_span!("peer", peer = %peer_id.fmt_short(), net = %network);
    let reader = async move {
        loop {
            // Wait for the next datagram, exiting on cancellation or connection
            // loss. Keeping the `select!` to "yield a datagram or return" leaves
            // the actual forwarding below at loop-body depth.
            let datagram = tokio::select! {
                _ = token.cancelled() => return,
                result = conn.read_datagram() => match result {
                    Ok(d) => d,
                    Err(e) => {
                        let intentional = matches!(
                            &e,
                            ConnectionError::ApplicationClosed(ac)
                                if ac.error_code == VarInt::from_u32(LEAVE_CODE)
                                    || ac.error_code == VarInt::from_u32(KICK_CODE)
                        );
                        tracing::warn!(peer = %peer_id.fmt_short(), ip = %peer_ip, error = %e, intentional, "peer connection lost");
                        let _ = disconnect_tx
                            .send(DisconnectEvent {
                                endpoint_id: peer_id,
                                ip: peer_ip,
                                ipv6: peer_ipv6,
                                network: network.clone(),
                                intentional,
                                conn_stable_id: Some(conn.stable_id()),
                            })
                            .await;
                        return;
                    }
                },
            };

            match evaluate_inbound(&datagram, peer_ip, peer_ipv6) {
                InboundDecision::Accept => {
                    stats.record_rx(datagram.len());
                    // Resolve the live writer for each packet: the sender is
                    // swapped on every TUN re-attach (VPN toggle). A send error
                    // means the writer is currently down (standby between a
                    // detach and the next attach); drop the packet and keep the
                    // reader alive so it forwards again once a new TUN attaches.
                    let _ = tun_tx.load_full().send(datagram).await;
                }
                InboundDecision::DropMalformed => stats.record_drop(DropReason::Malformed),
                InboundDecision::DropSpoof => {
                    stats.record_drop(DropReason::Spoof);
                    tracing::debug!(
                        peer = %peer_id.fmt_short(),
                        "dropped inbound packet with spoofed source IP"
                    );
                }
            }
        }
    };
    tokio::spawn(reader.instrument(span))
}

/// Spawns a task that consumes packets from `tun_rx` and writes them to the TUN
/// device. Single instance per session, serializes writes without a Mutex.
/// `active` is the data-plane gate: while it is false (standby, after `ray
/// down`) inbound datagrams are dropped instead of written, so a node that
/// stays connected to peers still carries no traffic.
pub fn spawn_tun_writer<W: crate::tun::TunWrite>(
    mut tun: W,
    mut tun_rx: mpsc::Receiver<Bytes>,
    active: Arc<AtomicBool>,
) -> JoinHandle<()> {
    use std::sync::atomic::Ordering;
    tokio::spawn(async move {
        while let Some(packet) = tun_rx.recv().await {
            if !active.load(Ordering::Relaxed) {
                // Data plane is down (standby). Drain and drop so the channel
                // never backs up while we keep the control plane connected.
                continue;
            }
            if let Err(e) = tun.write_packet(&packet).await {
                tracing::warn!(error = %e, "TUN write failed");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeTunWriter {
        written: std::sync::Arc<tokio::sync::Mutex<Vec<Vec<u8>>>>,
    }

    impl crate::tun::TunWrite for FakeTunWriter {
        async fn write_packet(&mut self, packet: &[u8]) -> anyhow::Result<()> {
            self.written.lock().await.push(packet.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn tun_writer_writes_when_active() {
        use std::sync::atomic::AtomicBool;
        let writer = FakeTunWriter::default();
        let sink = writer.written.clone();
        let (tx, rx) = mpsc::channel::<Bytes>(8);
        let active = std::sync::Arc::new(AtomicBool::new(true));
        let handle = spawn_tun_writer(writer, rx, active);
        tx.send(Bytes::from_static(b"kept")).await.unwrap();
        drop(tx); // close channel so the writer task exits
        handle.await.unwrap();
        let got = sink.lock().await;
        assert_eq!(got.as_slice(), &[b"kept".to_vec()]);
    }

    #[tokio::test]
    async fn tun_writer_drops_when_inactive() {
        use std::sync::atomic::AtomicBool;
        let writer = FakeTunWriter::default();
        let sink = writer.written.clone();
        let (tx, rx) = mpsc::channel::<Bytes>(8);
        let active = std::sync::Arc::new(AtomicBool::new(false));
        let handle = spawn_tun_writer(writer, rx, active);
        tx.send(Bytes::from_static(b"dropped")).await.unwrap();
        drop(tx);
        handle.await.unwrap();
        assert!(sink.lock().await.is_empty());
    }

    /// Mesh address the test packets are sourced from; passed to
    /// `evaluate_inbound` as the sending peer's assigned IP so the ingress
    /// anti-spoof check passes.
    const TEST_V4: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 5);
    const TEST_V6: Ipv6Addr = Ipv6Addr::UNSPECIFIED;

    fn make_tcp_packet(dst_port: u16) -> Vec<u8> {
        let mut p = vec![0u8; 24];
        p[0] = 0x45; // IPv4, IHL=5
        p[9] = 6; // TCP
        p[12..16].copy_from_slice(&[100, 64, 0, 5]); // src ip (TEST_V4)
        p[16..20].copy_from_slice(&[100, 64, 0, 3]); // dst ip
        p[20] = 0;
        p[21] = 80; // src port 80
        p[22] = (dst_port >> 8) as u8;
        p[23] = dst_port as u8;
        p
    }

    #[test]
    fn inbound_oversized_datagram_dropped_as_malformed() {
        let huge = vec![0u8; MAX_PEER_DATAGRAM + 1];
        assert!(matches!(
            evaluate_inbound(&huge, TEST_V4, TEST_V6),
            InboundDecision::DropMalformed
        ));
    }

    #[test]
    fn inbound_well_formed_packet_accepted() {
        // With the userspace firewall removed, a well-formed packet sourced from
        // the peer's own mesh IP is accepted; filtering is the host firewall's job.
        assert!(matches!(
            evaluate_inbound(&make_tcp_packet(443), TEST_V4, TEST_V6),
            InboundDecision::Accept
        ));
    }

    #[test]
    fn inbound_spoofed_source_ip_dropped() {
        // A packet whose source IP isn't the sending peer's assigned mesh IP is
        // dropped as spoofed, before any in-daemon listener sees it.
        let pkt = make_tcp_packet(80); // sourced from TEST_V4 (100.64.0.5)
        // Same packet, but the peer is supposedly assigned a different IP.
        assert!(matches!(
            evaluate_inbound(&pkt, Ipv4Addr::new(100, 64, 0, 9), TEST_V6),
            InboundDecision::DropSpoof
        ));
        // With the matching peer IP it passes.
        assert!(matches!(
            evaluate_inbound(&pkt, TEST_V4, TEST_V6),
            InboundDecision::Accept
        ));
    }

    #[test]
    fn magic_dns_predicate_matches_only_magic_ip_port_53() {
        let mk = |ip: IpAddr, port: u16| packet::PacketInfo {
            src_ip: "100.64.0.5".parse().unwrap(),
            dst_ip: ip,
            protocol: 17,
            src_port: 50000,
            dst_port: port,
            tcp_flags: 0,
            icmp_type: 0,
            icmp_id: 0,
        };
        assert!(is_magic_dns(&mk(IpAddr::V4(crate::dns::magic_dns_v4_node()), 53)));
        assert!(!is_magic_dns(&mk(IpAddr::V4(crate::dns::magic_dns_v4_node()), 80)));
        assert!(!is_magic_dns(&mk("100.64.0.9".parse().unwrap(), 53)));
    }
}

