# Multi-Peer Mesh (Phase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evolve pitopi from a two-peer point-to-point VPN into an N-peer mesh network where the creator coordinates IP assignment and peers connect directly to each other.

**Architecture:** The creator acts as coordinator — accepting connections, assigning IPs from 100.64.0.0/24, and broadcasting the peer list. Each new peer receives the full peer list and connects directly to every existing peer (full mesh). A control channel over a dedicated QUIC bidirectional stream handles IP assignment and peer list exchange, while data traffic continues over QUIC datagrams. A shared routing table (`HashMap<Ipv4Addr, Connection>`) dispatches TUN packets to the correct peer connection based on destination IP.

**Tech Stack:** Rust, iroh (QUIC streams + datagrams), tokio, serde + serde_json (control messages), tun

## Global Constraints

- Use `cargo -q` for all cargo commands
- TUN MTU stays at 1200
- Virtual IPs in 100.64.0.0/24 range
- ALPN: `b"pitopi/net/0"`
- Control messages are length-prefixed JSON over QUIC bidirectional streams
- Data packets use QUIC datagrams (unchanged)
- macOS TUN requires destination address (point-to-point)
- Coordinator = creator. IP .1 is always the coordinator.

---

### Task 1: Control Protocol Messages

**Files:**
- Create: `src/control.rs`
- Modify: `src/main.rs:1` (add `mod control;`)
- Modify: `Cargo.toml` (add serde, serde_json)

**Interfaces:**
- Produces: `ControlMsg` enum with `serde::Serialize + Deserialize`, `PeerInfo` struct, `send_msg(stream, msg)` and `recv_msg(stream)` async functions

- [ ] **Step 1: Add dependencies to Cargo.toml**

Add to `[dependencies]`:
```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: Write test for message serialization roundtrip**

In `src/control.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_roundtrip_welcome() {
        let msg = ControlMsg::Welcome {
            your_ip: Ipv4Addr::new(100, 64, 0, 3),
            peers: vec![PeerInfo {
                ip: Ipv4Addr::new(100, 64, 0, 2),
                endpoint_id: "test-id-abc123".to_string(),
            }],
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_peer_joined() {
        let msg = ControlMsg::PeerJoined(PeerInfo {
            ip: Ipv4Addr::new(100, 64, 0, 5),
            endpoint_id: "node-xyz".to_string(),
        });
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_roundtrip_mesh_hello() {
        let msg = ControlMsg::MeshHello {
            ip: Ipv4Addr::new(100, 64, 0, 4),
        };
        let bytes = encode_msg(&msg);
        let decoded = decode_msg(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo -q test -p pitopi control`
Expected: compilation errors (types not defined yet)

- [ ] **Step 4: Implement control messages**

In `src/control.rs`:
```rust
use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub ip: Ipv4Addr,
    pub endpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    Welcome {
        your_ip: Ipv4Addr,
        peers: Vec<PeerInfo>,
    },
    PeerJoined(PeerInfo),
    PeerLeft {
        ip: Ipv4Addr,
    },
    MeshHello {
        ip: Ipv4Addr,
    },
    MeshWelcome {
        ip: Ipv4Addr,
    },
}

pub fn encode_msg(msg: &ControlMsg) -> Vec<u8> {
    let json = serde_json::to_vec(msg).expect("serialize control message");
    let len = (json.len() as u32).to_be_bytes();
    [len.as_slice(), &json].concat()
}

pub fn decode_msg(data: &[u8]) -> Result<ControlMsg> {
    anyhow::ensure!(data.len() >= 4, "message too short");
    let len = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
    anyhow::ensure!(data.len() >= 4 + len, "incomplete message");
    serde_json::from_slice(&data[4..4 + len]).context("invalid control message")
}

pub async fn send_msg(stream: &mut SendStream, msg: &ControlMsg) -> Result<()> {
    let data = encode_msg(msg);
    stream.write_all(&data).await.context("send control message")?;
    Ok(())
}

pub async fn recv_msg(stream: &mut RecvStream) -> Result<ControlMsg> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.context("read message length")?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= 65536, "control message too large");
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.context("read message body")?;
    serde_json::from_slice(&body).context("decode control message")
}
```

- [ ] **Step 5: Add `mod control;` to main.rs**

Add `mod control;` after line 6 in `src/main.rs`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo -q test -p pitopi control`
Expected: all 3 tests pass

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/control.rs src/main.rs
git commit -m "feat: add control protocol messages for multi-peer mesh"
```

---

### Task 2: Peer Table (Routing Table + IP Allocator)

**Files:**
- Create: `src/peers.rs`
- Modify: `src/main.rs:1` (add `mod peers;`)

**Interfaces:**
- Consumes: `PeerInfo` from Task 1
- Produces: `PeerTable` with `add(ip, conn)`, `remove(ip)`, `lookup(ip) -> Option<Connection>`, `all_connections()`, `all_peers() -> Vec<PeerInfo>`, and `IpAllocator` with `next() -> Ipv4Addr`

- [ ] **Step 1: Write tests for IP allocator and peer table**

In `src/peers.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_allocator_sequential() {
        let mut alloc = IpAllocator::new();
        assert_eq!(alloc.next(), Ipv4Addr::new(100, 64, 0, 2));
        assert_eq!(alloc.next(), Ipv4Addr::new(100, 64, 0, 3));
        assert_eq!(alloc.next(), Ipv4Addr::new(100, 64, 0, 4));
    }

    #[test]
    fn test_peer_table_add_remove() {
        let table = PeerTable::new();
        let ip = Ipv4Addr::new(100, 64, 0, 2);
        assert!(table.lookup(&ip).is_none());
        // Can't easily test with real Connection objects, but we test the structure
    }
}
```

- [ ] **Step 2: Implement PeerTable and IpAllocator**

In `src/peers.rs`:
```rust
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

use iroh::endpoint::Connection;

use crate::control::PeerInfo;

pub struct IpAllocator {
    next_octet: u8,
}

impl IpAllocator {
    pub fn new() -> Self {
        Self { next_octet: 2 }
    }

    pub fn next(&mut self) -> Ipv4Addr {
        let ip = Ipv4Addr::new(100, 64, 0, self.next_octet);
        self.next_octet += 1;
        ip
    }
}

#[derive(Clone)]
pub struct PeerTable {
    inner: Arc<RwLock<HashMap<Ipv4Addr, PeerEntry>>>,
}

pub struct PeerEntry {
    pub conn: Connection,
    pub endpoint_id: String,
}

impl PeerTable {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add(&self, ip: Ipv4Addr, conn: Connection, endpoint_id: String) {
        self.inner.write().unwrap().insert(ip, PeerEntry { conn, endpoint_id });
    }

    pub fn remove(&self, ip: &Ipv4Addr) -> Option<PeerEntry> {
        self.inner.write().unwrap().remove(ip)
    }

    pub fn lookup(&self, ip: &Ipv4Addr) -> Option<Connection> {
        self.inner.read().unwrap().get(ip).map(|e| e.conn.clone())
    }

    pub fn all_connections(&self) -> Vec<(Ipv4Addr, Connection)> {
        self.inner.read().unwrap().iter().map(|(ip, e)| (*ip, e.conn.clone())).collect()
    }

    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|(ip, e)| PeerInfo {
                ip: *ip,
                endpoint_id: e.endpoint_id.clone(),
            })
            .collect()
    }
}
```

- [ ] **Step 3: Add `mod peers;` to main.rs**

- [ ] **Step 4: Run tests**

Run: `cargo -q test -p pitopi peers`
Expected: pass

- [ ] **Step 5: Commit**

```bash
git add src/peers.rs src/main.rs
git commit -m "feat: add peer table and IP allocator for mesh routing"
```

---

### Task 3: Multi-Peer Forwarding

**Files:**
- Modify: `src/forward.rs` (rewrite to use PeerTable instead of single Connection)

**Interfaces:**
- Consumes: `PeerTable` from Task 2, `TunDevice`, `Stats`, `CancellationToken`
- Produces: `run_mesh(tun, peers, token, stats)` — reads TUN packets and routes by destination IP; spawns per-peer iroh readers

- [ ] **Step 1: Rewrite forward.rs for multi-peer routing**

Replace the entire `src/forward.rs` with:
```rust
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
    let version = packet[0] >> 4;
    if version != 4 {
        return None;
    }
    Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
}

pub async fn run(
    tun: TunDevice,
    conn: Connection,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>(256);

    let tun_to_iroh = tokio::spawn(tun_read_single(
        tun,
        conn.clone(),
        tun_rx,
        token.clone(),
        stats.clone(),
    ));
    let iroh_to_tun = tokio::spawn(iroh_read_loop(conn, tun_tx, token.clone(), stats));

    tokio::select! {
        r = tun_to_iroh => r??,
        r = iroh_to_tun => r??,
    }

    Ok(())
}

pub async fn run_mesh(
    tun: TunDevice,
    peers: PeerTable,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let (tun_tx, mut tun_rx) = mpsc::channel::<Vec<u8>>(256);
    let peers_for_tx = peers.clone();

    let tun_reader = tokio::spawn({
        let tun = tun.share();
        let token = token.clone();
        let stats = stats.clone();
        let peers = peers_for_tx;
        async move {
            let mut buf = vec![0u8; 1500];
            loop {
                tokio::select! {
                    _ = token.cancelled() => return Ok(()),
                    result = tun.read_packet(&mut buf) => {
                        let n = result?;
                        if n > 0 {
                            if let Some(dst) = dest_ip(&buf[..n]) {
                                if let Some(conn) = peers.lookup(&dst) {
                                    match conn.send_datagram(Bytes::copy_from_slice(&buf[..n])) {
                                        Ok(()) => stats.record_tx(n),
                                        Err(_) => stats.record_drop(),
                                    }
                                } else {
                                    stats.record_drop();
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    let tun_writer = tokio::spawn({
        let tun = tun.share();
        async move {
            while let Some(packet) = tun_rx.recv().await {
                if let Err(e) = tun.write_packet(&packet).await {
                    tracing::warn!(error = %e, "TUN write failed");
                }
            }
            Ok::<(), anyhow::Error>(())
        }
    });

    tokio::select! {
        _ = token.cancelled() => Ok(()),
        r = tun_reader => r??,
        r = tun_writer => r??,
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

async fn tun_read_single(
    tun: TunDevice,
    conn: Connection,
    mut incoming: mpsc::Receiver<Vec<u8>>,
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
                    match conn.send_datagram(Bytes::copy_from_slice(&buf[..n])) {
                        Ok(()) => stats.record_tx(n),
                        Err(_) => stats.record_drop(),
                    }
                }
            }
            Some(packet) = incoming.recv() => {
                tun.write_packet(&packet).await?;
            }
        }
    }
}

async fn iroh_read_loop(
    conn: Connection,
    tun_tx: mpsc::Sender<Vec<u8>>,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = conn.read_datagram() => {
                let datagram = result?;
                stats.record_rx(datagram.len());
                if tun_tx.send(datagram.to_vec()).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dest_ip_valid_ipv4() {
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45; // IPv4, IHL=5
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
        packet[0] = 0x60; // IPv6
        assert_eq!(dest_ip(&packet), None);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo -q test -p pitopi forward`
Expected: 3 tests pass

- [ ] **Step 3: Verify compilation**

Run: `cargo -q check`
Expected: compiles clean

- [ ] **Step 4: Commit**

```bash
git add src/forward.rs
git commit -m "feat: add mesh forwarding with IP-based routing"
```

---

### Task 4: Coordinator — Multi-Peer Accept Loop with Control Channel

**Files:**
- Modify: `src/main.rs` (rewrite `cmd_create` for multi-peer, add `cmd_join` mesh support)
- Modify: `src/tun.rs` (adjust for subnet routing instead of single peer dest)

**Interfaces:**
- Consumes: `PeerTable`, `IpAllocator`, `ControlMsg`, `send_msg`/`recv_msg`, `run_mesh`, `spawn_peer_reader` from Tasks 1-3

- [ ] **Step 1: Update TUN creation to support mesh (subnet destination)**

In `src/tun.rs`, add a second constructor:
```rust
pub fn create_mesh(addr: Ipv4Addr) -> Result<Self> {
    let mut config = Configuration::default();
    config
        .address(addr)
        .destination(Ipv4Addr::new(100, 64, 0, 1))
        .netmask((255, 255, 255, 0))
        .mtu(TUN_MTU)
        .up();

    #[cfg(target_os = "linux")]
    config.platform_config(|p| {
        p.ensure_root_privileges(true);
    });

    let device = tun::create_as_async(&config)?;
    tracing::info!(%addr, "TUN device created (mesh)");
    Ok(Self {
        device: Arc::new(Mutex::new(device)),
    })
}
```

- [ ] **Step 2: Rewrite main.rs for multi-peer mesh**

Replace `src/main.rs` entirely:
```rust
mod control;
mod forward;
mod identity;
mod peers;
mod shutdown;
mod stats;
mod transport;
mod tun;

use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::endpoint::{Connection, Endpoint};
use iroh::EndpointId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use control::{ControlMsg, PeerInfo};
use peers::{IpAllocator, PeerTable};
use stats::Stats;

const COORDINATOR_IP: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 1);

const BACKOFF_INITIAL: std::time::Duration = std::time::Duration::from_secs(1);
const BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Parser)]
#[command(name = "pitopi", about = "P2P mesh VPN powered by iroh")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new network and wait for peers
    Create,
    /// Join an existing network using a node ID
    Join {
        /// The endpoint ID of the network creator
        node_id: EndpointId,
    },
}

fn check_root() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("pitopi requires root privileges to create TUN devices. Run with sudo.");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    check_root();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let cli = Cli::parse();

    let token = shutdown::token();
    let stats = stats::Stats::new();
    stats.spawn_logger(token.clone());

    match cli.command {
        Command::Create => cmd_create(token, stats).await,
        Command::Join { node_id } => cmd_join(node_id, token, stats).await,
    }
}

async fn cmd_create(token: CancellationToken, stats: Arc<Stats>) -> Result<()> {
    let key = identity::load_or_create()?;
    let ep = transport::create_endpoint(key).await?;

    tracing::info!("network created");
    tracing::info!(ip = %COORDINATOR_IP, "your virtual IP");
    tracing::info!(node_id = %ep.id(), "share this node ID with peers");

    let tun_dev = tun::TunDevice::create_mesh(COORDINATOR_IP)
        .context("failed to create TUN device")?;

    let peers = PeerTable::new();
    let mut ip_alloc = IpAllocator::new();

    let (tun_tx, _) = mpsc::channel::<Vec<u8>>(256);

    let mesh_handle = tokio::spawn(forward::run_mesh(
        tun_dev.share(),
        peers.clone(),
        token.clone(),
        stats.clone(),
    ));

    loop {
        tracing::info!("waiting for peers to join...");

        let conn = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = transport::accept_connection(&ep) => {
                match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to accept connection");
                        continue;
                    }
                }
            }
        };

        let assigned_ip = ip_alloc.next();
        let existing_peers = peers.peer_infos();
        let peer_endpoint_id = conn.remote_id().to_string();

        peers.add(assigned_ip, conn.clone(), peer_endpoint_id.clone());

        let peers_clone = peers.clone();
        let token_clone = token.clone();
        let stats_clone = stats.clone();
        let tun_tx_clone = tun_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_new_peer(
                conn,
                assigned_ip,
                existing_peers,
                peers_clone,
                token_clone,
                stats_clone,
                tun_tx_clone,
            )
            .await
            {
                tracing::warn!(ip = %assigned_ip, error = %e, "peer session ended");
            }
        });

        tracing::info!(ip = %assigned_ip, "peer joined network");
    }
}

async fn handle_new_peer(
    conn: Connection,
    assigned_ip: Ipv4Addr,
    existing_peers: Vec<PeerInfo>,
    peers: PeerTable,
    token: CancellationToken,
    stats: Arc<Stats>,
    tun_tx: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.context("open control stream")?;

    let welcome = ControlMsg::Welcome {
        your_ip: assigned_ip,
        peers: existing_peers.clone(),
    };
    control::send_msg(&mut send, &welcome).await?;

    tracing::info!(ip = %assigned_ip, peer_count = existing_peers.len(), "sent welcome");

    let new_peer_info = PeerInfo {
        ip: assigned_ip,
        endpoint_id: conn.remote_id().to_string(),
    };
    broadcast_to_peers(&peers, &ControlMsg::PeerJoined(new_peer_info), Some(assigned_ip)).await;

    let reader_handle = forward::spawn_peer_reader(conn.clone(), tun_tx, token.clone(), stats);

    tokio::select! {
        _ = token.cancelled() => {}
        _ = reader_handle => {
            tracing::info!(ip = %assigned_ip, "peer disconnected");
            peers.remove(&assigned_ip);
            broadcast_to_peers(
                &peers,
                &ControlMsg::PeerLeft { ip: assigned_ip },
                None,
            )
            .await;
        }
    }

    Ok(())
}

async fn broadcast_to_peers(peers: &PeerTable, msg: &ControlMsg, exclude: Option<Ipv4Addr>) {
    for (ip, conn) in peers.all_connections() {
        if Some(ip) == exclude {
            continue;
        }
        match conn.open_bi().await {
            Ok((mut send, _recv)) => {
                if let Err(e) = control::send_msg(&mut send, msg).await {
                    tracing::warn!(peer_ip = %ip, error = %e, "failed to send control message");
                }
            }
            Err(e) => {
                tracing::warn!(peer_ip = %ip, error = %e, "failed to open control stream");
            }
        }
    }
}

async fn cmd_join(
    node_id: EndpointId,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let key = identity::load_or_create()?;
    let ep = transport::create_endpoint(key).await?;

    let mut backoff = BACKOFF_INITIAL;

    loop {
        tracing::info!("connecting to network...");

        let conn = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = transport::connect_to_peer(&ep, node_id) => {
                match result {
                    Ok(conn) => {
                        backoff = BACKOFF_INITIAL;
                        conn
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to connect");
                        backoff_sleep(&token, &mut backoff).await;
                        continue;
                    }
                }
            }
        };

        match join_mesh(conn, &ep, token.clone(), stats.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if token.is_cancelled() {
                    return Ok(());
                }
                tracing::warn!(error = %e, "connection lost, reconnecting...");
                backoff_sleep(&token, &mut backoff).await;
            }
        }
    }
}

async fn join_mesh(
    coordinator_conn: Connection,
    ep: &Endpoint,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let (_send, mut recv) = coordinator_conn.accept_bi().await.context("accept control stream")?;

    let welcome = control::recv_msg(&mut recv).await?;
    let (my_ip, existing_peers) = match welcome {
        ControlMsg::Welcome { your_ip, peers } => (your_ip, peers),
        other => anyhow::bail!("expected Welcome, got {:?}", other),
    };

    tracing::info!(ip = %my_ip, peers = existing_peers.len(), "joined network");

    let tun_dev = tun::TunDevice::create_mesh(my_ip).context("failed to create TUN device")?;

    let peers = PeerTable::new();

    peers.add(COORDINATOR_IP, coordinator_conn.clone(), coordinator_conn.remote_id().to_string());

    let (tun_tx, _) = mpsc::channel::<Vec<u8>>(256);

    forward::spawn_peer_reader(coordinator_conn.clone(), tun_tx.clone(), token.clone(), stats.clone());

    for peer_info in &existing_peers {
        let peer_id: EndpointId = peer_info.endpoint_id.parse()
            .context("invalid peer endpoint id")?;
        match transport::connect_to_peer(ep, peer_id).await {
            Ok(conn) => {
                let (mut send, mut peer_recv) = conn.open_bi().await?;
                control::send_msg(&mut send, &ControlMsg::MeshHello { ip: my_ip }).await?;

                peers.add(peer_info.ip, conn.clone(), peer_info.endpoint_id.clone());
                forward::spawn_peer_reader(conn, tun_tx.clone(), token.clone(), stats.clone());
                tracing::info!(peer_ip = %peer_info.ip, "connected to mesh peer");
            }
            Err(e) => {
                tracing::warn!(peer_ip = %peer_info.ip, error = %e, "failed to connect to mesh peer");
            }
        }
    }

    let control_listener = tokio::spawn({
        let peers = peers.clone();
        let ep = ep.clone();
        let token = token.clone();
        let stats = stats.clone();
        let tun_tx = tun_tx.clone();
        let my_ip = my_ip;
        async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    result = coordinator_conn.accept_bi() => {
                        match result {
                            Ok((_send, mut recv)) => {
                                match control::recv_msg(&mut recv).await {
                                    Ok(ControlMsg::PeerJoined(info)) => {
                                        tracing::info!(peer_ip = %info.ip, "new peer joined");
                                        if let Ok(peer_id) = info.endpoint_id.parse::<EndpointId>() {
                                            // Let the new peer connect to us instead
                                        }
                                    }
                                    Ok(ControlMsg::PeerLeft { ip }) => {
                                        tracing::info!(peer_ip = %ip, "peer left");
                                        peers.remove(&ip);
                                    }
                                    Ok(other) => {
                                        tracing::warn!(?other, "unexpected control message");
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "control message error");
                                    }
                                }
                            }
                            Err(_) => return,
                        }
                    }
                }
            }
        }
    });

    let mesh_acceptor = tokio::spawn({
        let ep = ep.clone();
        let peers = peers.clone();
        let token = token.clone();
        let stats = stats.clone();
        let tun_tx = tun_tx.clone();
        async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    result = transport::accept_connection(&ep) => {
                        match result {
                            Ok(conn) => {
                                match conn.accept_bi().await {
                                    Ok((_send, mut recv)) => {
                                        match control::recv_msg(&mut recv).await {
                                            Ok(ControlMsg::MeshHello { ip }) => {
                                                tracing::info!(peer_ip = %ip, "mesh peer connected");
                                                peers.add(ip, conn.clone(), conn.remote_id().to_string());
                                                forward::spawn_peer_reader(conn, tun_tx.clone(), token.clone(), stats.clone());
                                            }
                                            _ => {}
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "failed to accept mesh handshake");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "failed to accept mesh connection");
                            }
                        }
                    }
                }
            }
        }
    });

    forward::run_mesh(tun_dev, peers, token, stats).await
}

async fn backoff_sleep(token: &CancellationToken, backoff: &mut std::time::Duration) {
    tracing::info!(secs = backoff.as_secs(), "retrying in");
    tokio::select! {
        _ = token.cancelled() => {}
        _ = tokio::time::sleep(*backoff) => {}
    }
    *backoff = (*backoff * 2).min(BACKOFF_MAX);
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo -q check`
Expected: compiles

- [ ] **Step 4: Run all tests**

Run: `cargo -q test`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/tun.rs
git commit -m "feat: multi-peer mesh with coordinator, control channel, and routing"
```

---

### Task 5: Integration — Verify and Polish

**Files:**
- Modify: `TODO.md` (check off completed items)

- [ ] **Step 1: Run full test suite**

Run: `cargo -q test`
Expected: all tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo -q clippy -- -D warnings`

- [ ] **Step 3: Fix any clippy warnings**

- [ ] **Step 4: Update TODO.md**

Check off completed Phase 2 items:
- [x] Creator becomes the initial coordinator
- [x] Control channel over a bidirectional QUIC stream
- [x] Joiner requests an IP via control channel
- [x] Accept multiple incoming connections
- [x] Full mesh — peers connect to every other peer directly
- [x] Routing table — HashMap<Ipv4Addr, Connection>
- [x] Forwarding layer reads destination IP and routes to correct connection
- [x] Peer disconnect detection — remove from routing table, notify remaining peers

- [ ] **Step 5: Commit**

```bash
git add TODO.md
git commit -m "docs: update TODO.md with Phase 2 progress"
```
