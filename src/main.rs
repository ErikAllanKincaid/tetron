mod acl;
mod audit;
mod config;
mod control;
mod forward;
mod identity;
mod peers;
mod room_code;
mod shutdown;
mod stats;
mod transport;
mod tun;

use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use iroh::endpoint::Endpoint;
use iroh::EndpointId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use control::{ControlMsg, PeerInfo};
use peers::{IpAllocator, PeerTable};
use stats::Stats;

fn coordinator_ip(subnet_index: u8) -> Ipv4Addr {
    tun::TunDevice::coordinator_ip(subnet_index)
}

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
    Create {
        /// Network name (defaults to "default")
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// Join an existing network using a node ID or room code
    Join {
        /// The endpoint ID or room code of the network creator
        node_id: String,
        /// Network name (defaults to "default")
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// List saved networks
    List,
    /// Leave a network (remove from saved config)
    Leave {
        /// Name of the network to leave
        name: String,
    },
    /// Show status of active networks
    Status,
    /// Connect to all saved networks
    Up,
    /// Disconnect from all networks
    Down,
    /// Install system service (systemd on Linux, launchd on macOS)
    InstallService,
    /// Uninstall system service
    UninstallService,
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
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
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let cli = Cli::parse();

    match cli.command {
        Command::List => cmd_list(),
        Command::Leave { name } => cmd_leave(&name),
        Command::Create { name } => {
            check_root();
            let token = shutdown::token();
            let stats = stats::Stats::new();
            stats.spawn_logger(token.clone());
            cmd_create(&name, token, stats).await
        }
        Command::Join { node_id, name } => {
            check_root();
            let node_id = room_code::parse_node_id(&node_id)
                .context("invalid node ID or room code")?;
            let token = shutdown::token();
            let stats = stats::Stats::new();
            stats.spawn_logger(token.clone());
            cmd_join(node_id, &name, token, stats).await
        }
        Command::Status => cmd_status(),
        Command::Up => {
            check_root();
            let token = shutdown::token();
            let stats = stats::Stats::new();
            stats.spawn_logger(token.clone());
            cmd_up(token, stats).await
        }
        Command::Down => cmd_down(),
        Command::InstallService => cmd_install_service(),
        Command::UninstallService => cmd_uninstall_service(),
        Command::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "pitopi",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

async fn cmd_create(name: &str, token: CancellationToken, stats: Arc<Stats>) -> Result<()> {
    let key = identity::load_or_create()?;
    let alpn = transport::network_alpn(name);
    let ep = transport::create_endpoint_with_alpns(key, vec![alpn.clone()]).await?;

    // Assign subnet index
    let mut app_config = config::load()?;
    let subnet_index = config::next_subnet_index(&app_config);

    cmd_create_on_endpoint(name, &ep, subnet_index, token, stats, &mut app_config).await
}

/// Run a coordinator for a single network on a shared endpoint.
async fn cmd_create_on_endpoint(
    name: &str,
    ep: &Endpoint,
    subnet_index: u8,
    token: CancellationToken,
    stats: Arc<Stats>,
    app_config: &mut config::AppConfig,
) -> Result<()> {
    let coord_ip = coordinator_ip(subnet_index);
    let room_code = room_code::encode(&ep.id());
    let alpn = transport::network_alpn(name);

    tracing::info!(name = %name, subnet_index, "network created");
    tracing::info!(ip = %coord_ip, "your virtual IP");
    tracing::info!(node_id = %ep.id(), "share this node ID with peers");
    tracing::info!(room_code = %room_code, "or share this room code");

    // Save network to config
    config::upsert_network(
        app_config,
        config::NetworkConfig {
            name: name.to_string(),
            coordinator_id: ep.id().to_string(),
            assigned_ip: None,
            subnet_index,
            peers: vec![],
        },
    );
    config::save(app_config)?;
    tracing::info!("saved network to config");

    let tun_dev = tun::TunDevice::create_mesh_subnet(coord_ip, subnet_index)
        .context("failed to create TUN device")?;

    let peers = PeerTable::new();
    let mut ip_alloc = IpAllocator::for_subnet(subnet_index);
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
        tracing::info!(network = %name, "waiting for peers to join...");

        let conn = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = transport::accept_connection_with_alpn(ep) => {
                match result {
                    Ok((conn, conn_alpn)) => {
                        if conn_alpn != alpn {
                            continue;
                        }
                        conn
                    }
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

        tracing::info!(ip = %assigned_ip, network = %name, "peer joined network");

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
    name: &str,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let key = identity::load_or_create()?;
    let alpn = transport::network_alpn(name);
    let ep = transport::create_endpoint_with_alpns(key, vec![alpn.clone()]).await?;

    // Assign subnet index
    let app_config = config::load()?;
    let subnet_index = config::next_subnet_index(&app_config);

    cmd_join_on_endpoint(node_id, name, &ep, subnet_index, token, stats).await
}

/// Join a network on a shared endpoint with a specific subnet and ALPN.
async fn cmd_join_on_endpoint(
    node_id: EndpointId,
    name: &str,
    ep: &Endpoint,
    subnet_index: u8,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let alpn = transport::network_alpn(name);
    let mut backoff = BACKOFF_INITIAL;

    loop {
        tracing::info!(network = %name, "connecting to network...");

        let conn = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            result = transport::connect_to_peer_with_alpn(ep, node_id, &alpn) => {
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

        match join_mesh(conn, ep, name, node_id, subnet_index, token.clone(), stats.clone()).await {
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
    network_name: &str,
    coordinator_node_id: EndpointId,
    subnet_index: u8,
    token: CancellationToken,
    stats: Arc<Stats>,
) -> Result<()> {
    let alpn = transport::network_alpn(network_name);
    let coord_ip = coordinator_ip(subnet_index);

    let (_send, mut recv) = coordinator_conn
        .accept_bi()
        .await
        .context("accept control stream from coordinator")?;

    let welcome = control::recv_msg(&mut recv).await?;
    let (my_ip, existing_peers) = match welcome {
        ControlMsg::Welcome { your_ip, peers } => (your_ip, peers),
        other => anyhow::bail!("expected Welcome, got {:?}", other),
    };

    tracing::info!(ip = %my_ip, network = %network_name, peers = existing_peers.len(), "joined network");

    // Save network membership to config
    let peer_entries: Vec<config::PeerEntry> = existing_peers
        .iter()
        .map(|p| config::PeerEntry {
            ip: p.ip,
            endpoint_id: p.endpoint_id.clone(),
        })
        .collect();
    let mut app_config = config::load()?;
    config::upsert_network(
        &mut app_config,
        config::NetworkConfig {
            name: network_name.to_string(),
            coordinator_id: coordinator_node_id.to_string(),
            assigned_ip: Some(my_ip),
            subnet_index,
            peers: peer_entries,
        },
    );
    config::save(&app_config)?;
    tracing::info!(name = %network_name, "saved network to config");

    let tun_dev = tun::TunDevice::create_mesh_subnet(my_ip, subnet_index)
        .context("failed to create TUN device")?;

    let peers = PeerTable::new();
    let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>(256);

    forward::spawn_tun_writer(tun_dev.share(), tun_rx);

    peers.add(
        coord_ip,
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
        match transport::connect_to_peer_with_alpn(ep, peer_id, &alpn).await {
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
        let expected_alpn = alpn.clone();
        async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    result = transport::accept_connection_with_alpn(&ep) => {
                        match result {
                            Ok((conn, conn_alpn)) => {
                                // Strict isolation: only accept connections for our network
                                if conn_alpn != expected_alpn {
                                    tracing::debug!(
                                        expected = %String::from_utf8_lossy(&expected_alpn),
                                        got = %String::from_utf8_lossy(&conn_alpn),
                                        "ignoring connection for different network"
                                    );
                                    continue;
                                }
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

fn cmd_list() -> Result<()> {
    let app_config = config::load()?;
    if app_config.networks.is_empty() {
        println!("No saved networks.");
        return Ok(());
    }
    for net in &app_config.networks {
        let ip_str = net
            .assigned_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "coordinator".to_string());
        println!(
            "{} (coordinator: {}, ip: {}, peers: {})",
            net.name,
            net.coordinator_id,
            ip_str,
            net.peers.len()
        );
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    let app_config = config::load()?;
    if app_config.networks.is_empty() {
        println!("No networks configured.");
        return Ok(());
    }
    println!("Networks:");
    for net in &app_config.networks {
        let role = if net.assigned_ip.is_none() {
            "coordinator"
        } else {
            "member"
        };
        let ip_str = net
            .assigned_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| coordinator_ip(net.subnet_index).to_string());
        println!("  {} [{}]", net.name, role);
        println!("    IP: {}", ip_str);
        println!("    Subnet: 100.64.{}.0/24", net.subnet_index);
        println!("    Coordinator: {}", net.coordinator_id);
        if !net.peers.is_empty() {
            println!("    Peers:");
            for peer in &net.peers {
                println!("      {} ({})", peer.ip, peer.endpoint_id);
            }
        }
    }
    Ok(())
}

fn cmd_leave(name: &str) -> Result<()> {
    let mut app_config = config::load()?;
    if config::remove_network(&mut app_config, name) {
        config::save(&app_config)?;
        println!("Left network '{}'.", name);
    } else {
        println!("Network '{}' not found.", name);
    }
    Ok(())
}

async fn cmd_up(token: CancellationToken, stats: Arc<Stats>) -> Result<()> {
    let app_config = config::load()?;
    if app_config.networks.is_empty() {
        println!("No saved networks. Use 'pitopi create' or 'pitopi join' first.");
        return Ok(());
    }

    let key = identity::load_or_create()?;

    // Collect all ALPNs upfront so the shared endpoint serves all networks
    let alpns: Vec<Vec<u8>> = app_config
        .networks
        .iter()
        .map(|net| transport::network_alpn(&net.name))
        .collect();

    let ep = transport::create_endpoint_with_alpns(key, alpns).await?;

    let mut handles = Vec::new();
    for net in &app_config.networks {
        let subnet_index = net.subnet_index;
        let alpn = transport::network_alpn(&net.name);

        if net.assigned_ip.is_some() {
            let coordinator_id: EndpointId = net.coordinator_id.parse()
                .context("invalid coordinator id in config")?;
            let name = net.name.clone();
            let ep = ep.clone();
            let token = token.clone();
            let stats = stats.clone();
            handles.push(tokio::spawn(async move {
                tracing::info!(network = %name, subnet_index, "connecting...");
                match transport::connect_to_peer_with_alpn(&ep, coordinator_id, &alpn).await {
                    Ok(conn) => {
                        if let Err(e) = join_mesh(conn, &ep, &name, coordinator_id, subnet_index, token, stats).await {
                            tracing::warn!(network = %name, error = %e, "disconnected");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(network = %name, error = %e, "failed to connect");
                    }
                }
            }));
        } else {
            let name = net.name.clone();
            let ep = ep.clone();
            let token = token.clone();
            let stats = stats.clone();
            handles.push(tokio::spawn(async move {
                tracing::info!(network = %name, subnet_index, "starting coordinator...");
                let mut app_config = config::load().unwrap_or_default();
                if let Err(e) = cmd_create_on_endpoint(&name, &ep, subnet_index, token, stats, &mut app_config).await {
                    tracing::warn!(network = %name, error = %e, "coordinator stopped");
                }
            }));
        }
    }

    tokio::select! {
        _ = token.cancelled() => {}
        _ = futures::future::join_all(handles) => {}
    }

    Ok(())
}

fn cmd_down() -> Result<()> {
    println!("Stopping all networks. Send SIGTERM to the running pitopi process.");
    Ok(())
}

fn cmd_install_service() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let service = include_str!("../contrib/pitopi.service");
        let path = std::path::Path::new("/etc/systemd/system/pitopi.service");
        std::fs::write(path, service)?;
        println!("Installed systemd service to {}", path.display());
        println!("Run: sudo systemctl enable --now pitopi");
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let plist = include_str!("../contrib/com.pitopi.vpn.plist");
        let path = std::path::Path::new("/Library/LaunchDaemons/com.pitopi.vpn.plist");
        std::fs::write(path, plist)?;
        println!("Installed launchd daemon to {}", path.display());
        println!("Run: sudo launchctl load {}", path.display());
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        anyhow::bail!("service installation not supported on this platform");
    }
}

fn cmd_uninstall_service() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let path = std::path::Path::new("/etc/systemd/system/pitopi.service");
        if path.exists() {
            std::fs::remove_file(path)?;
            println!("Removed systemd service.");
            println!("Run: sudo systemctl daemon-reload");
        } else {
            println!("Service not installed.");
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let path = std::path::Path::new("/Library/LaunchDaemons/com.pitopi.vpn.plist");
        if path.exists() {
            println!("Run: sudo launchctl unload {}", path.display());
            std::fs::remove_file(path)?;
            println!("Removed launchd daemon.");
        } else {
            println!("Service not installed.");
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        anyhow::bail!("service uninstallation not supported on this platform");
    }
}

async fn backoff_sleep(token: &CancellationToken, backoff: &mut std::time::Duration) {
    tracing::info!(secs = backoff.as_secs(), "retrying in");
    tokio::select! {
        _ = token.cancelled() => {}
        _ = tokio::time::sleep(*backoff) => {}
    }
    *backoff = (*backoff * 2).min(BACKOFF_MAX);
}
