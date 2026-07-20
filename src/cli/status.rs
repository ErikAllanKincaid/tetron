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
        Some("start the service: sudo tetron install".into())
    } else if m.contains("expired") || m.contains("invite") {
        Some("ask the coordinator for a fresh code: tetron invite <net>".into())
    } else if m.contains("root") || m.contains("permission") || m.contains("operator") {
        Some("run with sudo, or `sudo tetron set-operator <you>` once".into())
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

/// Render rows into real left-aligned columns: each column padded to the
/// widest cell in that column (header included), so e.g. a row of IP
/// addresses actually lines up under each other -- unlike `table()` above,
/// which deliberately does not align columns. Returns each line without a
/// leading indent; callers prepend their own.
fn render_aligned_table(headers: &[&str], rows: &[Vec<String>]) -> Vec<String> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.len());
            }
        }
    }
    let pad_row = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{:<width$}", c, width = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    let mut lines = vec![pad_row(&header_cells)];
    lines.extend(rows.iter().map(|row| pad_row(row)));
    lines
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
            let _ = (packets_rx, packets_tx);
            let state = if active { "active" } else { "standby" };
            let cli_version = env!("CARGO_PKG_VERSION");
            let shown_version = if daemon_version.is_empty() {
                cli_version
            } else {
                daemon_version.as_str()
            };
            println!();
            println!(
                "  tetron v{}  state {}  endpoint {}",
                shown_version,
                state,
                endpoint_id.fmt_short(),
            );
            println!(
                "    traffic  ↑{}  ↓{}",
                format_bytes(bytes_tx),
                format_bytes(bytes_rx),
            );
            if !active {
                println!("  (run `tetron resume` to activate)");
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

            // Daemon/CLI version skew: after a manual binary upgrade the CLI is
            // new but the long-running daemon may still be the old one (e.g. it
            // was never restarted). Empty `daemon_version` means the daemon
            // predates this field — say nothing rather than guess.
            if !daemon_version.is_empty() && daemon_version != cli_version {
                println!();
                println!(
                    "  ! daemon is v{} but CLI is v{}",
                    daemon_version, cli_version,
                );
                println!("  (run `sudo tetron restart` to restart the daemon onto the new binary)");
            }
            println!();
        }
        ipc::IpcMessage::Error { message } => print_error("status failed", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}

/// Render one network block: header (name, subnet, admin/member counts,
/// interface), the network key (admins only), and an aligned role/host/ip/via
/// table with the local node as its own `(you)` row first.
fn print_network(net: &ipc::NetworkStatus) {
    let am_i_admin = net.role.is_coordinator();
    let online = net.peers.iter().filter(|p| p.connection.is_some()).count();
    let admins_total =
        net.peers.iter().filter(|p| p.is_coordinator).count() + if am_i_admin { 1 } else { 0 };
    let admins_online = net
        .peers
        .iter()
        .filter(|p| p.is_coordinator && p.connection.is_some())
        .count()
        + if am_i_admin { 1 } else { 0 };

    println!();
    print!(
        "  network {}   subnet {}   admins {admins_online}/{admins_total}   members {online}/{}",
        net.name,
        net.subnet,
        net.peers.len(),
    );
    if !net.tun_name.is_empty() && net.tun_name != "pending" {
        print!("   interface {}", net.tun_name);
    }
    if !net.active {
        print!("   ·standby·");
    }
    println!();

    // network_key: only shown to admins -- a plain member can't act on it
    // (`nuke`/`kick` would reject them regardless of whether they know the
    // value), so showing it to non-admins is pure clutter. Short prefix only
    // (nuke/kick both accept an unambiguous >=10-char prefix); the full value
    // remains available via `--json` for everyone regardless of role.
    let short_id: Option<String> = if am_i_admin {
        net.network_key.as_ref().map(|key| key.chars().take(10).collect())
    } else {
        None
    };
    if let Some(ref short) = short_id {
        println!("    network_key {short}");
    }

    println!();
    let headers = ["role", "host", "ip", "via"];
    let mut rows: Vec<Vec<String>> = Vec::new();
    let my_host = net
        .my_hostname
        .clone()
        .unwrap_or_else(|| net.my_ip.to_string());
    rows.push(vec![
        (if am_i_admin { "admin" } else { "member" }).to_string(),
        my_host,
        net.my_ip.to_string(),
        "(you)".to_string(),
    ]);
    for peer in &net.peers {
        let host = peer.hostname.clone().unwrap_or_else(|| peer.ip.to_string());
        let via = match &peer.connection {
            Some(ci) => match ci.conn_type {
                ipc::ConnType::Direct => "direct",
                ipc::ConnType::Relay => "relay",
                ipc::ConnType::Tor => "tor",
                ipc::ConnType::Unknown => "?",
            },
            None => "offline",
        };
        rows.push(vec![
            (if peer.is_coordinator { "admin" } else { "member" }).to_string(),
            host,
            peer.ip.to_string(),
            via.to_string(),
        ]);
    }
    for line in render_aligned_table(&headers, &rows) {
        println!("    {line}");
    }

    // NUKE-CONSENSUS: pending proposals, so members see one being considered
    // before it executes. The actionable "run tetron nuke ..." suggestion
    // needs the network_key value, so it's only included for admins (who
    // have `short_id`) -- a non-admin still sees that a proposal exists, just
    // without a command they couldn't use anyway.
    if !net.nuke_proposals.is_empty() {
        let ids: Vec<&str> = net
            .nuke_proposals
            .iter()
            .map(|p| p.short_id.as_str())
            .collect();
        match short_id.as_deref() {
            Some(id_hint) => println!(
                "    ! nuke proposed by {} ({}/2) — run `tetron nuke {id_hint}` to second, or `tetron nuke {id_hint} --cancel` to withdraw yours",
                ids.join(", "),
                net.nuke_proposals.len(),
            ),
            None => println!(
                "    ! nuke proposed by {} ({}/2)",
                ids.join(", "),
                net.nuke_proposals.len(),
            ),
        }
    }
}

/// `tetron standby`: put the daemon on standby (tear down the TUN, revert DNS,
/// drop connections) while leaving the daemon process running so `tetron
/// resume` can reactivate it without root.
pub(crate) async fn ipc_standby(network: Option<String>) -> Result<()> {
    let mut stream = ipc::connect().await?;
    ipc::send(&mut stream, ipc::IpcMessage::Standby { network }).await?;
    let resp = ipc::recv(&mut stream).await?;
    match resp {
        ipc::IpcMessage::Ok { message } => println!("{}", message),
        ipc::IpcMessage::Error { message } => print_error("error", &message, None),
        other => eprintln!("Unexpected response: {:?}", other),
    }
    Ok(())
}
