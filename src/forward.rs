//! Mesh packet forwarding between TUN device and peer QUIC connections.
//!
//! Three concurrent tasks handle the data plane:
//! - [`run_mesh`]: reads outgoing packets from TUN, routes to correct peer via [`PeerTable`]
//! - [`spawn_peer_reader`]: one per peer, reads incoming datagrams and forwards to TUN writer
//! - [`spawn_tun_writer`]: single task, writes incoming packets to the TUN device

use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use dashmap::DashMap;
use iroh::EndpointId;
use iroh::endpoint::Connection;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::acl::AclData;
use crate::firewall::{self, Action, Direction, SharedFirewall};
use crate::peers::PeerTable;
use crate::stats::Stats;
use crate::tun::{TunReader, TunWriter};

#[derive(Clone)]
pub struct SharedAcl {
    inner: Arc<DashMap<String, AclData>>,
}

impl SharedAcl {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub fn set(&self, network: &str, acl: AclData) {
        self.inner.insert(network.to_string(), acl);
    }

    pub fn remove(&self, network: &str) {
        self.inner.remove(network);
    }

    pub fn get(&self, network: &str) -> AclData {
        self.inner
            .get(network)
            .map(|e| e.value().clone())
            .unwrap_or_else(AclData::empty)
    }
}

/// Sent by [`spawn_peer_reader`] when a peer connection drops,
/// consumed by the reconnect loop (joiner) or cleanup task (coordinator).
pub struct DisconnectEvent {
    pub endpoint_id: EndpointId,
    pub ip: Ipv4Addr,
}

/// Main TUN read loop. Reads packets from the TUN device, extracts the destination IP,
/// looks up the peer in [`PeerTable`], and sends the packet as a QUIC datagram.
/// Packets with no matching peer are silently dropped.
pub async fn run_mesh(
    mut tun: TunReader,
    peers: PeerTable,
    local_id: EndpointId,
    shared_acl: SharedAcl,
    firewall: SharedFirewall,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let mut buf = vec![0u8; 1500];
    loop {
        tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = tun.read_packet(&mut buf) => {
                let n = result?;
                if n > 0 {
                    tracing::debug!(len = n, first_byte = buf[0], "TUN read");
                    let pkt = &buf[..n];
                    if let Some(info) = firewall::parse_packet_info(pkt) {
                        if let Some((conn, peer_endpoint_id, network)) = peers.lookup_full(&info.dst_ip) {
                            let acl = shared_acl.get(&network);
                            if !acl.is_allowed(&local_id, &peer_endpoint_id) {
                                tracing::debug!(dst = %info.dst_ip, "ACL denied outbound");
                                stats.record_drop();
                                continue;
                            }
                            if firewall.evaluate(Direction::Out, info.protocol, info.dst_port, &peer_endpoint_id) == Action::Deny {
                                tracing::debug!(dst = %info.dst_ip, port = info.dst_port, "firewall denied outbound");
                                stats.record_drop();
                                continue;
                            }
                            tracing::debug!(dst = %info.dst_ip, "routing to peer");
                            match conn.send_datagram(Bytes::copy_from_slice(pkt)) {
                                Ok(()) => stats.record_tx(n),
                                Err(e) => {
                                    tracing::debug!(dst = %info.dst_ip, error = %e, "datagram send failed");
                                    stats.record_drop();
                                }
                            }
                        } else {
                            tracing::debug!(dst = %info.dst_ip, "no peer for dst");
                            stats.record_drop();
                        }
                    } else {
                        tracing::debug!(len = n, "not IPv4, dropping");
                    }
                }
            }
        }
    }
}

/// Spawns a task that reads QUIC datagrams from a single peer connection and
/// forwards them to the TUN writer via `tun_tx`. On connection loss, sends a
/// [`DisconnectEvent`] and exits.
#[allow(clippy::too_many_arguments)]
pub fn spawn_peer_reader(
    conn: Connection,
    peer_id: EndpointId,
    peer_ip: Ipv4Addr,
    local_id: EndpointId,
    network: String,
    shared_acl: SharedAcl,
    firewall: SharedFirewall,
    tun_tx: mpsc::Sender<Vec<u8>>,
    disconnect_tx: mpsc::Sender<DisconnectEvent>,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                result = conn.read_datagram() => {
                    match result {
                        Ok(datagram) => {
                            if !shared_acl.get(&network).is_allowed(&peer_id, &local_id) {
                                stats.record_drop();
                                continue;
                            }
                            if let Some(info) = firewall::parse_packet_info(&datagram)
                                && firewall.evaluate(Direction::In, info.protocol, info.dst_port, &peer_id) == Action::Deny {
                                    stats.record_drop();
                                    continue;
                            }
                            stats.record_rx(datagram.len());
                            if tun_tx.send(datagram.to_vec()).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(peer = %peer_id.fmt_short(), ip = %peer_ip, error = %e, "peer connection lost");
                            let _ = disconnect_tx.send(DisconnectEvent { endpoint_id: peer_id, ip: peer_ip }).await;
                            return;
                        }
                    }
                }
            }
        }
    })
}

/// Spawns a task that consumes packets from `tun_rx` and writes them to the TUN device.
/// Single instance per session — serializes writes without a Mutex.
pub fn spawn_tun_writer(
    mut tun: TunWriter,
    mut tun_rx: mpsc::Receiver<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(packet) = tun_rx.recv().await {
            if let Err(e) = tun.write_packet(&packet).await {
                tracing::warn!(error = %e, "TUN write failed");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_packet_valid_ipv4() {
        let mut packet = vec![0u8; 24];
        packet[0] = 0x45;
        packet[9] = 6; // TCP
        packet[16] = 100;
        packet[17] = 64;
        packet[18] = 0;
        packet[19] = 3;
        let info = firewall::parse_packet_info(&packet).unwrap();
        assert_eq!(info.dst_ip, Ipv4Addr::new(100, 64, 0, 3));
        assert_eq!(info.protocol, 6);
    }

    #[test]
    fn test_parse_packet_too_short() {
        assert!(firewall::parse_packet_info(&[0x45; 10]).is_none());
    }

    #[test]
    fn test_parse_packet_not_ipv4() {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x60;
        assert!(firewall::parse_packet_info(&packet).is_none());
    }
}
