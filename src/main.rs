mod daemon;
mod dht;
mod config;
mod control;
mod forward;
mod identity;
mod ipc;
mod membership;
mod peers;
mod room_code;
mod shutdown;
mod stats;
mod transport;
mod tun;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

use futures::StreamExt;
use iroh::endpoint::{PathEvent, Connection as IrohConnection};

use membership::GroupMode;

/// Logs iroh path events (opened, closed, selected) for a peer connection.
pub(crate) fn spawn_path_logger(conn: IrohConnection, label: String) {
    let paths = conn.paths();
    for path in paths.iter() {
        tracing::info!(
            peer = %label,
            addr = ?path.remote_addr(),
            rtt = ?path.rtt(),
            selected = path.is_selected(),
            "existing path"
        );
    }

    tokio::spawn(async move {
        let mut events = conn.path_events();
        while let Some(event) = events.next().await {
            match event {
                PathEvent::Opened { remote_addr, .. } => {
                    tracing::info!(peer = %label, addr = ?remote_addr, "path opened");
                }
                PathEvent::Closed { remote_addr, .. } => {
                    tracing::info!(peer = %label, addr = ?remote_addr, "path closed");
                }
                PathEvent::Selected { remote_addr, .. } => {
                    tracing::info!(peer = %label, addr = ?remote_addr, "path selected");
                }
                PathEvent::Lagged { missed, .. } => {
                    tracing::warn!(peer = %label, missed, "path events lagged");
                }
                _ => {}
            }
        }
    });
}

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
        /// Membership mode: open or restricted
        #[arg(long, default_value = "restricted")]
        mode: GroupMode,
    },
    /// Join an existing network using a node ID or room code
    Join {
        /// The endpoint ID or room code of the network creator
        node_id: String,
        /// Network name (override the name from room code)
        #[arg(long)]
        name: Option<String>,
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
    /// Start the daemon (manages all networks, listens for IPC commands)
    Daemon,
    /// Connect to all saved networks (alias for daemon)
    Up,
    /// Disconnect from all networks (signals daemon to shut down)
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
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();

    match cli.command {
        Command::List => cmd_list(),
        Command::Leave { name } => ipc_leave(&name).await,
        Command::Create { name, mode } => ipc_create(&name, mode).await,
        Command::Join { node_id, name } => ipc_join(&node_id, name.as_deref()).await,
        Command::Status => ipc_status().await,
        Command::Daemon | Command::Up => {
            check_root();
            let token = shutdown::token();
            let stats = stats::Stats::new();
            stats.spawn_logger(token.clone());
            daemon::run_daemon(token, stats).await
        }
        Command::Down => ipc_down().await,
        Command::InstallService => cmd_install_service(),
        Command::UninstallService => cmd_uninstall_service(),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "pitopi", &mut std::io::stdout());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Client-side commands (no daemon needed)
// ---------------------------------------------------------------------------

fn cmd_list() -> Result<()> {
    let app_config = config::load()?;
    if app_config.networks.is_empty() {
        println!("No saved networks.");
        return Ok(());
    }
    for net in &app_config.networks {
        let ip_str = net
            .my_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "coordinator".to_string());
        println!(
            "{} (coordinator: {}, ip: {}, members: {}, mode: {:?})",
            net.name,
            net.coordinator_id,
            ip_str,
            net.members.len(),
            net.group_mode,
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// IPC client commands (require daemon running)
// ---------------------------------------------------------------------------

async fn ipc_create(name: &str, mode: GroupMode) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send_msg(&mut stream, &ipc::IpcRequest::Create {
        name: name.to_string(),
        mode,
    }).await?;
    let resp: ipc::IpcResponse = ipc::recv_msg(&mut stream).await?;
    match resp {
        ipc::IpcResponse::Created { name, room_code, my_ip } => {
            println!("Network '{}' created.", name);
            println!("  IP: {}", my_ip);
            println!("  Room code: {}", room_code);
        }
        ipc::IpcResponse::Error { message } => {
            eprintln!("Error: {}", message);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

async fn ipc_join(node_id: &str, name: Option<&str>) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send_msg(&mut stream, &ipc::IpcRequest::Join {
        node_id: node_id.to_string(),
        name: name.map(String::from),
    }).await?;
    let resp: ipc::IpcResponse = ipc::recv_msg(&mut stream).await?;
    match resp {
        ipc::IpcResponse::Joined { name, my_ip } => {
            println!("Joined network '{}'.", name);
            println!("  IP: {}", my_ip);
        }
        ipc::IpcResponse::Error { message } => {
            eprintln!("Error: {}", message);
        }
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

async fn ipc_leave(name: &str) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send_msg(&mut stream, &ipc::IpcRequest::Leave {
        name: name.to_string(),
    }).await?;
    let resp: ipc::IpcResponse = ipc::recv_msg(&mut stream).await?;
    match resp {
        ipc::IpcResponse::Ok { message } => println!("{}", message),
        ipc::IpcResponse::Error { message } => eprintln!("Error: {}", message),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

async fn ipc_status() -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send_msg(&mut stream, &ipc::IpcRequest::Status).await?;
    let resp: ipc::IpcResponse = ipc::recv_msg(&mut stream).await?;
    match resp {
        ipc::IpcResponse::Status { endpoint_id, networks } => {
            println!("Endpoint: {}", endpoint_id);
            if networks.is_empty() {
                println!("No active networks.");
            } else {
                for net in &networks {
                    let role = match &net.role {
                        ipc::NetworkRole::Coordinator => "coordinator",
                        ipc::NetworkRole::Member => "member",
                    };
                    println!("  {} [{}]", net.name, role);
                    println!("    IP: {}", net.my_ip);
                    if !net.peers.is_empty() {
                        println!("    Peers:");
                        for peer in &net.peers {
                            println!("      {} ({})", peer.ip, peer.endpoint_id);
                        }
                    }
                }
            }
        }
        ipc::IpcResponse::Error { message } => eprintln!("Error: {}", message),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

async fn ipc_down() -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send_msg(&mut stream, &ipc::IpcRequest::Shutdown).await?;
    let resp: ipc::IpcResponse = ipc::recv_msg(&mut stream).await?;
    match resp {
        ipc::IpcResponse::Ok { message } => println!("{}", message),
        ipc::IpcResponse::Error { message } => eprintln!("Error: {}", message),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Service install/uninstall
// ---------------------------------------------------------------------------

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
