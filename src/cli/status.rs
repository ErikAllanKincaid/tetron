//! CLI status & diagnostics output plus shared presentation helpers
//! (`table`, `print_error`, …): status, down, set-hostname.

use crate::*;

/// Human-readable byte size (GiB/MiB/KiB/B) for traffic and transfer counters.
pub(crate) fn format_bytes(b: u64) -> String {
    bytesize::ByteSize(b).to_string()
}

/// Render a plain error block to stderr:
/// ```text
///   ! <title>
///     <detail>
///     hint  <hint>
/// ```
/// When `hint` is `None`, a hint is inferred from common daemon error strings.
pub(crate) fn print_error(title: &str, detail: &str, hint: Option<&str>) {
    eprintln!("  ! {title}");
    if !detail.is_empty() {
        eprintln!("    {detail}");
    }
    let hint = hint.map(str::to_string).or_else(|| infer_hint(detail));
    if let Some(h) = hint {
        eprintln!("    hint  {h}");
    }
}

/// Print a JSON value to stdout (used by `--json` on every list command).
pub(crate) fn print_json(value: &serde_json::Value) {
    println!("{value}");
}

/// Map a daemon error message to an actionable hint, best-effort.
pub(crate) fn infer_hint(message: &str) -> Option<String> {
    let m = message.to_lowercase();
    if m.contains("daemon") && (m.contains("not running") || m.contains("connect")) {
        Some("start the service: sudo torpedo up".into())
    } else if m.contains("expired") || m.contains("invite") {
        Some("ask the coordinator for a fresh code: torpedo invite <net>".into())
    } else if m.contains("root") || m.contains("permission") || m.contains("operator") {
        Some("run with sudo, or `sudo torpedo set-operator <you>` once".into())
    } else if m.contains("hostname") && m.contains("collision") {
        Some("pick another name: --hostname <name>".into())
    } else {
        None
    }
}

/// Render a "next steps" footer.
pub(crate) fn print_next(steps: &[(&str, &str)]) {
    for (i, (cmd, blurb)) in steps.iter().enumerate() {
        if i == 0 {
            println!("    next  {cmd}   {blurb}");
        } else {
            println!("          {cmd}   {blurb}");
        }
    }
}

/// Standard borderless table: indented `pad` spaces. No column alignment in
/// plain mode; each row is printed as space-separated values.
pub(crate) fn table(headers: &[&str], rows: Vec<Vec<String>>, pad: usize) -> String {
    let mut out = String::new();
    // Header row
    out.push_str(&indent(&headers.join("  "), pad));
    out.push('\n');
    // Body rows
    for row in &rows {
        out.push_str(&indent(&row.join("  "), pad));
        out.push('\n');
    }
    out
}

/// Prefix every line of `block` with `indent` spaces (for nested table output).
pub(crate) fn indent(block: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    block
        .lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Naively pluralize `noun` for a count (append `s` unless `n == 1`). The count
/// itself is shown separately, so this returns just the noun. Good enough for
/// the status pending summary's nouns.
pub(crate) fn pluralize(n: usize, noun: &str) -> String {
    if n == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}

pub(crate) async fn ipc_status() -> Result<()> {
    let Ok(mut stream) = ipc::connect().await else {
        // Daemon not running — show saved config
        let app_config = config::load()?;
        println!();
            println!("  ! daemon not running");
        if app_config.networks.is_empty() {
            println!("  (no saved networks)");
            println!();
            return Ok(());
        }
        println!("  saved networks:");
        for net in &app_config.networks {
            let ip_str = net
                .my_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "?".to_string());
            println!(
                "    {}  ({})  {} members",
                net.name,
                ip_str,
                net.members.len()
            );
        }
        println!();
        return Ok(());
    };

    ipc::send(&mut stream, ipc::IpcMessage::Status).await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::StatusResponse {
            endpoint_id,
            active,
            daemon_version,
            networks,
            packets_rx,
            packets_tx,
            bytes_rx,
            bytes_tx,
            ..
        } => {
            if json_enabled() {
                print_json(&serde_json::json!({
                    "endpoint": endpoint_id.to_string(),
                    "active": active,
                    "daemon_version": daemon_version,
                    "networks": networks,
                    "traffic": {
                        "packets_rx": packets_rx, "packets_tx": packets_tx,
                        "bytes_rx": bytes_rx, "bytes_tx": bytes_tx,
                    },
                }));
                return Ok(());
            }
            let _ = (packets_rx, packets_tx, bytes_rx, bytes_tx);
            let state = if active { "up" } else { "standby" };
            println!();
            println!(
                "  torpedo  {}      endpoint {}",
                state,
                endpoint_id.fmt_short(),
            );
            if !active {
                println!("  (run `torpedo up` to activate)");
            }

            if networks.is_empty() {
                println!();
                println!("  (no active networks)");
            } else {
                for net in &networks {
                    print_network(net);
                }
            }

            // Show inactive networks from config that the daemon didn't restore
            let active_names: std::collections::HashSet<&str> =
                networks.iter().map(|n| n.name.as_str()).collect();
            if let Ok(app_config) = config::load() {
                let inactive: Vec<_> = app_config
                    .networks
                    .iter()
                    .filter(|n| !active_names.contains(n.name.as_str()))
                    .collect();
                for net in &inactive {
                    println!();
                    println!("  {}  ·inactive·", net.name);
                }
            }

            print_pending_summary(&networks);

            // Daemon/CLI version skew: after a manual binary upgrade the CLI is
            // new but the long-running daemon may still be the old one (e.g. it
            // was never restarted). Empty `daemon_version` means the daemon
            // predates this field — say nothing rather than guess.
            let cli_version = env!("CARGO_PKG_VERSION");
            if !daemon_version.is_empty() && daemon_version != cli_version {
                println!();
                println!(
                    "  ! daemon is v{} but CLI is v{}",
                    daemon_version,
                    cli_version,
                );
                println!(
                    "  (run `sudo torpedo restart` to restart the daemon onto the new binary)"
                );
            }
            println!();
        }
        ipc::IpcMessage::Error { message } => print_error("status failed", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

/// Render one network block: header (name, role, hostname, ip, member count),
/// the peer list, and the shareable join code.
fn print_network(net: &ipc::NetworkStatus) {
    let role = net.role.to_string();
    let dns_name = net.my_hostname.clone();
    let online = net.peers.iter().filter(|p| p.connection.is_some()).count();
    println!();
    print!("  {}  ·{role}·", net.name);
    if let Some(ref dns) = dns_name {
        print!("   {dns}");
    }
    print!("   {}", net.my_ip);
    print!("   members {online}/{}", net.peers.len());
    println!();

    // Peer rows as text lines
    if net.peers.is_empty() {
        println!("    (no other members)");
    } else {
        for peer in &net.peers {
            let line = render_peer_line(peer);
            println!("    {line}");
        }
    }

    // join code
    if let Some(ref key) = net.network_key
        && !net.role.is_direct()
    {
        println!("    join {key}");
    }
}

/// Build one peer's status line (host, ipv4, via, rtt, tx, rx).
fn render_peer_line(peer: &ipc::PeerStatus) -> String {
    let host = peer
        .hostname
        .clone()
        .unwrap_or_else(|| peer.ip.to_string());
    match &peer.connection {
        Some(ci) => {
            let via = match ci.conn_type {
                ipc::ConnType::Direct => "direct",
                ipc::ConnType::Relay => "relay",
                ipc::ConnType::Tor => "tor",
                ipc::ConnType::Unknown => "?",
            };
            let rtt = match ci.rtt_ms {
                Some(ms) => format!("{ms:.0}ms"),
                None => "—".into(),
            };
            let up = format_bytes(ci.bytes_tx);
            let down = format_bytes(ci.bytes_rx);
            format!("{host}  {}  {via}  {rtt}  ↑{up}  ↓{down}", peer.ip)
        }
        None => format!("{host}  {}  —  offline", peer.ip),
    }
}

/// Render the trailing "pending" summary: things waiting on the user, each with
/// the command that clears it. Per-network items (join requests) name their
/// network.
fn print_pending_summary(networks: &[ipc::NetworkStatus]) {
    let mut pending: Vec<(usize, String, String)> = Vec::new();
    for net in networks {
        if net.pending_requests > 0 {
            pending.push((
                net.pending_requests,
                pluralize(net.pending_requests, "join request"),
                format!("torpedo requests {}", net.name),
            ));
        }
    }
    if pending.is_empty() {
        return;
    }
    println!();
    println!("  pending");
    for (n, what, cmd) in &pending {
        println!("    ({n}) {what}  {cmd}");
    }
}

/// `torpedo down`: put the daemon on standby (tear down the TUN, revert DNS, drop
/// connections) while leaving the daemon process running so `torpedo up` can
/// reactivate it without root.
pub(crate) async fn ipc_down() -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(&mut stream, ipc::IpcMessage::Down).await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
