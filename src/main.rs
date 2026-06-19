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
use iroh::endpoint::Endpoint;
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

    let tun_dev =
        tun::TunDevice::create_mesh(COORDINATOR_IP).context("failed to create TUN device")?;

    let peers = PeerTable::new();
    let mut ip_alloc = IpAllocator::new();
    let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>(256);

    forward::spawn_tun_writer(tun_dev.share(), tun_rx);

    tokio::spawn(forward::run_mesh(
        tun_dev,
        peers.clone(),
        tun_tx.clone(),
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

        peers.add(assigned_ip, conn.clone(), peer_endpoint_id);

        tracing::info!(ip = %assigned_ip, "peer joined network");

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
    }
}

async fn handle_new_peer(
    conn: iroh::endpoint::Connection,
    assigned_ip: Ipv4Addr,
    existing_peers: Vec<PeerInfo>,
    peers: PeerTable,
    token: CancellationToken,
    stats: Arc<Stats>,
    tun_tx: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let (mut send, _recv) = conn.open_bi().await.context("open control stream")?;

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

    let reader_handle =
        forward::spawn_peer_reader(conn.clone(), tun_tx, token.clone(), stats);

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
                    tracing::warn!(peer_ip = %ip, error = %e, "failed to broadcast");
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
    coordinator_conn: iroh::endpoint::Connection,
    ep: &Endpoint,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let (_send, mut recv) = coordinator_conn
        .accept_bi()
        .await
        .context("accept control stream from coordinator")?;

    let welcome = control::recv_msg(&mut recv).await?;
    let (my_ip, existing_peers) = match welcome {
        ControlMsg::Welcome { your_ip, peers } => (your_ip, peers),
        other => anyhow::bail!("expected Welcome, got {:?}", other),
    };

    tracing::info!(ip = %my_ip, peers = existing_peers.len(), "joined network");

    let tun_dev = tun::TunDevice::create_mesh(my_ip).context("failed to create TUN device")?;

    let peers = PeerTable::new();
    let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>(256);

    forward::spawn_tun_writer(tun_dev.share(), tun_rx);

    peers.add(
        COORDINATOR_IP,
        coordinator_conn.clone(),
        coordinator_conn.remote_id().to_string(),
    );
    forward::spawn_peer_reader(
        coordinator_conn.clone(),
        tun_tx.clone(),
        token.clone(),
        stats.clone(),
    );

    for peer_info in &existing_peers {
        let peer_id: EndpointId = peer_info
            .endpoint_id
            .parse()
            .context("invalid peer endpoint id")?;
        match transport::connect_to_peer(ep, peer_id).await {
            Ok(conn) => {
                let (mut send, _peer_recv) = conn.open_bi().await?;
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

    let _control_listener = tokio::spawn({
        let coordinator_conn = coordinator_conn.clone();
        let peers = peers.clone();
        let token = token.clone();
        async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    result = coordinator_conn.accept_bi() => {
                        match result {
                            Ok((_send, mut recv)) => {
                                match control::recv_msg(&mut recv).await {
                                    Ok(ControlMsg::PeerJoined(info)) => {
                                        tracing::info!(peer_ip = %info.ip, "new peer announced");
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

    let _mesh_acceptor = tokio::spawn({
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
                                        if let Ok(ControlMsg::MeshHello { ip }) = control::recv_msg(&mut recv).await {
                                            tracing::info!(peer_ip = %ip, "mesh peer connected");
                                            peers.add(ip, conn.clone(), conn.remote_id().to_string());
                                            forward::spawn_peer_reader(conn, tun_tx.clone(), token.clone(), stats.clone());
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "mesh handshake failed");
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

    forward::run_mesh(tun_dev, peers, tun_tx, token, stats).await
}

async fn backoff_sleep(token: &CancellationToken, backoff: &mut std::time::Duration) {
    tracing::info!(secs = backoff.as_secs(), "retrying in");
    tokio::select! {
        _ = token.cancelled() => {}
        _ = tokio::time::sleep(*backoff) => {}
    }
    *backoff = (*backoff * 2).min(BACKOFF_MAX);
}
