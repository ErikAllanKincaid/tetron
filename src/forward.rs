use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use iroh::endpoint::Connection;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::peers::PeerTable;
use crate::stats::Stats;
use crate::tun::TunDevice;

fn dest_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 {
        return None;
    }
    if packet[0] >> 4 != 4 {
        return None;
    }
    Some(Ipv4Addr::new(
        packet[16], packet[17], packet[18], packet[19],
    ))
}

pub async fn run_mesh(
    tun: TunDevice,
    peers: PeerTable,
    _tun_tx: mpsc::Sender<Vec<u8>>,
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
                    if let Some(dst) = dest_ip(&buf[..n]) {
                        if let Some(conn) = peers.lookup(&dst) {
                            tracing::debug!(%dst, "routing to peer");
                            match conn.send_datagram(Bytes::copy_from_slice(&buf[..n])) {
                                Ok(()) => stats.record_tx(n),
                                Err(_) => stats.record_drop(),
                            }
                        } else {
                            tracing::debug!(%dst, "no peer for dst");
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

pub fn spawn_peer_reader(
    conn: Connection,
    tun_tx: mpsc::Sender<Vec<u8>>,
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
                            stats.record_rx(datagram.len());
                            if tun_tx.send(datagram.to_vec()).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            }
        }
    })
}

pub fn spawn_tun_writer(
    tun: TunDevice,
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
    fn test_dest_ip_valid_ipv4() {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45;
        packet[16] = 100;
        packet[17] = 64;
        packet[18] = 0;
        packet[19] = 3;
        assert_eq!(dest_ip(&packet), Some(Ipv4Addr::new(100, 64, 0, 3)));
    }

    #[test]
    fn test_dest_ip_too_short() {
        assert_eq!(dest_ip(&[0x45; 10]), None);
    }

    #[test]
    fn test_dest_ip_not_ipv4() {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x60;
        assert_eq!(dest_ip(&packet), None);
    }
}
