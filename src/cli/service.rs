//! CLI service-management handlers: up, install, start/stop/restart, uninstall,
//! operator, plus small process/daemon-reachability helpers.

use crate::*;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Create the `tetron` system group if it doesn't already exist (Linux).
/// Best-effort: the daemon's config writer falls back to `root:root` ownership
/// when the group is missing, so a failure here only loosens the group-read
/// posture, never breaks startup.
#[cfg(target_os = "linux")]
pub(crate) fn ensure_tetron_group() {
    // `getent group tetron` exits 0 if the group exists.
    let exists = Command::new("getent")
        .args(["group", "tetron"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !exists {
        let _ = Command::new("groupadd")
            .args(["--system", "tetron"])
            .status();
    }
}

/// Strip the `" (deleted)"` marker Linux appends to `/proc/self/exe` once the
/// running binary's inode has been unlinked — e.g. after a manual upgrade that
/// replaces the installed binary while the old one is still running. Without
/// this strip a subsequent unit rewrite would get
/// `ExecStart=/usr/local/bin/tetron (deleted) daemon` and the service would
/// crash-loop with `unrecognized subcommand '(deleted)'`.
pub(crate) fn strip_deleted_suffix(path: &str) -> &str {
    path.strip_suffix(" (deleted)").unwrap_or(path)
}

/// Write the system service unit/plist, substituting the path of the binary
/// currently running so the service execs the same binary the user invoked
/// (rather than a hardcoded /usr/local/bin/tetron). Idempotent — safe to call on
/// every `tetron up`, keeping the exec path fresh if the binary moves.
#[allow(unused_variables)]
pub(crate) fn ensure_service_installed() -> Result<()> {
    let exe = std::env::current_exe()
        .context("failed to determine current executable path")?
        .to_string_lossy()
        .into_owned();
    let exe = strip_deleted_suffix(&exe).to_owned();

    #[cfg(target_os = "linux")]
    {
        // Ensure the `tetron` system group exists before the daemon writes its
        // config tree under /etc/tetron (owned root:tetron). Idempotent;
        // best-effort — the daemon falls back to root:root if the group is
        // absent (see config::set_owner).
        ensure_tetron_group();
        let path = Path::new("/etc/systemd/system/tetron.service");
        let service =
            include_str!("../../contrib/tetron.service").replace("/usr/local/bin/tetron", &exe);
        std::fs::write(path, service)
            .with_context(|| format!("failed to write {}", path.display()))?;
        run_cmd("systemctl", &["daemon-reload"]);
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let path = Path::new("/Library/LaunchDaemons/com.tetron.vpn.plist");
        // RENAME-008: match the plist's /usr/local/bin/tetron placeholder (was
        // the stale pre-fork /usr/local/bin/ray, which the plist no longer
        // contains — leaving the real exe path unsubstituted). Mirrors Linux.
        let plist = include_str!("../../contrib/com.tetron.vpn.plist")
            .replace("/usr/local/bin/tetron", &exe);
        std::fs::write(path, plist)
            .with_context(|| format!("failed to write {}", path.display()))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        anyhow::bail!("system service not supported on this platform");
    }
}

/// `tetron up`: activate the VPN.
///
/// If the daemon is already running (the common case — the system service
/// starts it at boot), this is just an unprivileged IPC call asking the daemon
/// to bring the TUN up, configure DNS, and reconnect networks. Only when no
/// daemon is reachable do we fall back to installing/starting the system
/// service, which requires root.
pub(crate) async fn cmd_up(hostname: Option<String>, network: Option<String>) -> Result<()> {
    if let Ok(mut stream) = ipc::connect().await {
        ipc::send(&mut stream, ipc::IpcMessage::Up { hostname, network }).await?;
        match ipc::recv(&mut stream).await? {
            ipc::IpcMessage::Ok { message } => println!("{message}"),
            ipc::IpcMessage::Error { message } => print_error("error", &message, None),
            other => eprintln!("Unexpected response: {other:?}"),
        }
        return Ok(());
    }

    // No daemon reachable — install and start the system service (needs root).
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "tetron service is not running. Start it with: sudo tetron up\n\
             (the daemon needs root to install the system service and create the TUN device)"
        );
        std::process::exit(1);
    }
    // A fresh service start always brings every network up (run_daemon's own
    // boot-time activate(None, None) already covers it) -- --network doesn't
    // apply when there was no daemon running to scope down from.
    install_and_start_service(hostname).await
}

/// Install/refresh the system service and (re)start it. Requires root.
///
/// Starting the service is fire-and-forget at the OS level, so we then wait for
/// the daemon to actually accept an IPC connection before declaring success. If
/// it never comes up (e.g. it crashed on a port/route conflict with another
/// VPN), we surface the tail of its log so the user knows what went wrong
/// instead of seeing a cheerful "started" followed by a dead `tetron status`.
pub(crate) async fn install_and_start_service(hostname: Option<String>) -> Result<()> {
    ensure_service_installed()?;

    #[cfg(target_os = "linux")]
    {
        run_cmd("systemctl", &["enable", "tetron"]);
        run_cmd("systemctl", &["restart", "tetron"]);
    }

    #[cfg(target_os = "macos")]
    {
        let path = "/Library/LaunchDaemons/com.tetron.vpn.plist";
        // Tear down any previously loaded job (e.g. one pointing at a stale
        // binary path) before loading the freshly written plist.
        run_cmd_quiet("launchctl", &["unload", path]);
        run_cmd("launchctl", &["load", "-w", path]);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("system service not supported on this platform");
    }

    // Wait for the freshly started daemon to accept IPC, then activate the VPN.
    eprintln!("waiting for daemon…");
    let daemon = wait_for_daemon(DAEMON_REACHABLE_TIMEOUT).await;
    match daemon {
        Some(mut stream) => {
            ipc::send(
                &mut stream,
                ipc::IpcMessage::Up {
                    hostname,
                    network: None,
                },
            )
            .await?;
            match ipc::recv(&mut stream).await? {
                ipc::IpcMessage::Ok { message } => println!("tetron service started. {message}"),
                ipc::IpcMessage::Error { message } => print_error("error", &message, None),
                other => eprintln!("Unexpected response: {other:?}"),
            }
            // We're root here (installing the service). Grant the invoking user
            // operator access so they can run `ray` without sudo from now on,
            // the way `tailscale up --operator=$USER` does.
            grant_operator_to_invoking_user().await;
            Ok(())
        }
        None => {
            eprintln!(
                "tetron service was started but the daemon never became reachable.\n\
                 It likely crashed on startup — common causes are the chosen overlay subnet\n\
                 overlapping an existing local network (see `tetron config set subnet`),\n\
                 DNS port 53 already in use, or a conflicting route."
            );
            print_daemon_log_tail();
            std::process::exit(1);
        }
    }
}

/// When the service is (re)installed under `sudo`, grant the invoking user
/// (`$SUDO_USER`) operator access so subsequent `ray` commands work without
/// root. Best-effort: silent if there is no `$SUDO_USER` or the daemon refuses.
pub(crate) async fn grant_operator_to_invoking_user() {
    let Ok(user) = std::env::var("SUDO_USER") else {
        return;
    };
    if user == "root" {
        return;
    }
    let Some(uid) = uid_for_user(&user) else {
        return;
    };
    if let Ok(mut stream) = ipc::connect().await {
        let _ = ipc::send(&mut stream, ipc::IpcMessage::SetOperator { uid }).await;
        if let Ok(ipc::IpcMessage::Ok { .. }) = ipc::recv(&mut stream).await {
            println!("granted operator access to '{user}' — run tetron without sudo");
        }
    }
}

/// Ensure the process is running as root for service-manager operations.
/// Prints a clear `sudo` hint and exits non-zero otherwise.
pub(crate) fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "this command manages the system service and needs root.\n\
             Re-run with: sudo tetron <command>"
        );
        std::process::exit(1);
    }
    Ok(())
}

/// `tetron install`: install the system service if needed (or refresh an existing
/// install), then start it and verify the daemon comes up. Requires root.
pub(crate) async fn cmd_install() -> Result<()> {
    require_root()?;
    install_and_start_service(None).await
}

/// Whether the system service unit/plist is installed on this host.
pub(crate) fn service_unit_exists() -> bool {
    #[cfg(target_os = "linux")]
    {
        return Path::new("/etc/systemd/system/tetron.service").exists();
    }
    #[cfg(target_os = "macos")]
    {
        return Path::new("/Library/LaunchDaemons/com.tetron.vpn.plist").exists();
    }
    #[allow(unreachable_code)]
    false
}

/// Restart the installed service via the OS service manager (without rewriting
/// the unit file) and wait for the daemon to accept IPC again. Backs
/// `tetron restart`; mirrors the `up`/`install` diagnostics.
#[allow(unreachable_code)]
pub(crate) async fn restart_service_and_wait() -> Result<()> {
    #[cfg(target_os = "linux")]
    run_cmd("systemctl", &["restart", "tetron"]);

    #[cfg(target_os = "macos")]
    run_cmd("launchctl", &["kickstart", "-k", "system/com.tetron.vpn"]);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("system service not supported on this platform");

    match wait_for_daemon(DAEMON_REACHABLE_TIMEOUT).await {
        Some(_) => {
            println!("tetron service restarted.");
            Ok(())
        }
        None => {
            eprintln!("tetron service was restarted but the daemon never became reachable.");
            print_daemon_log_tail();
            std::process::exit(1);
        }
    }
}

/// `tetron restart`: restart the already-installed system service via the OS
/// service manager (does not rewrite the unit file). Requires root. The daemon
/// comes back up active.
pub(crate) async fn cmd_restart() -> Result<()> {
    require_root()?;
    if !service_unit_exists() {
        eprintln!("tetron service is not installed. Run: sudo tetron up");
        std::process::exit(1);
    }
    restart_service_and_wait().await
}

/// `tetron stop`: stop the installed system service so the daemon exits and all
/// peer connections close cleanly (a clean offline, distinct from `tetron down`
/// standby). Does not disable or uninstall the unit. Requires root.
#[allow(unreachable_code)]
pub(crate) async fn cmd_stop() -> Result<()> {
    require_root()?;
    if !service_unit_exists() {
        eprintln!("tetron service is not installed. Nothing to stop.");
        std::process::exit(1);
    }

    #[cfg(target_os = "linux")]
    run_cmd("systemctl", &["stop", "tetron"]);

    #[cfg(target_os = "macos")]
    run_cmd(
        "launchctl",
        &["unload", "/Library/LaunchDaemons/com.tetron.vpn.plist"],
    );

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("system service not supported on this platform");

    println!("tetron service stopped.");
    Ok(())
}

/// `tetron start`: start the already-installed system service via the OS service
/// manager and wait for the daemon to accept IPC. The daemon comes back up with
/// the control and data planes on. Requires root.
#[allow(unreachable_code)]
pub(crate) async fn cmd_start() -> Result<()> {
    require_root()?;
    if !service_unit_exists() {
        eprintln!("tetron service is not installed. Run: sudo tetron up");
        std::process::exit(1);
    }

    #[cfg(target_os = "linux")]
    run_cmd("systemctl", &["start", "tetron"]);

    #[cfg(target_os = "macos")]
    run_cmd(
        "launchctl",
        &["load", "-w", "/Library/LaunchDaemons/com.tetron.vpn.plist"],
    );

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("system service not supported on this platform");

    match wait_for_daemon(DAEMON_REACHABLE_TIMEOUT).await {
        Some(_) => {
            println!("tetron service started.");
            Ok(())
        }
        None => {
            eprintln!("tetron service was started but the daemon never became reachable.");
            print_daemon_log_tail();
            std::process::exit(1);
        }
    }
}

/// How long to wait for a freshly (re)started daemon to accept IPC before
/// declaring it unreachable. Must comfortably exceed the service manager's
/// stop-then-relaunch latency (SIGTERM → exit → respawn); the old 8s value was
/// shorter than an ungraceful shutdown could take, so a healthy daemon was
/// reported as "never became reachable" and a re-run would kill the one that
/// had just come up.
pub(crate) const DAEMON_REACHABLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Poll the IPC socket until the daemon answers or the deadline passes.
pub(crate) async fn wait_for_daemon(timeout: Duration) -> Option<ipc::IpcFramed> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(stream) = ipc::connect().await {
            return Some(stream);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Print the last few lines of the daemon log so a failed startup is diagnosable.
pub(crate) fn print_daemon_log_tail() {
    #[cfg(target_os = "macos")]
    {
        let path = "/var/log/tetron.log";
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let tail: Vec<&str> = contents.lines().rev().take(15).collect();
                if tail.is_empty() {
                    eprintln!("\n(daemon log {path} is empty)");
                } else {
                    eprintln!("\nLast lines of {path}:");
                    for line in tail.into_iter().rev() {
                        eprintln!("  {line}");
                    }
                }
            }
            Err(e) => eprintln!("\n(could not read daemon log {path}: {e})"),
        }
    }

    #[cfg(target_os = "linux")]
    {
        eprintln!("\nRecent daemon log (journalctl -u tetron):");
        run_cmd("journalctl", &["-u", "tetron", "-n", "15", "--no-pager"]);
    }
}

#[allow(dead_code)]
pub(crate) fn run_cmd(program: &str, args: &[&str]) {
    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("warning: `{program}` exited with {status}"),
        Err(e) => eprintln!("warning: failed to run `{program}`: {e}"),
    }
}

/// Run a command, ignoring its exit status (used for best-effort teardown).
#[allow(dead_code)]
pub(crate) fn run_cmd_quiet(program: &str, args: &[&str]) {
    let _ = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub(crate) fn cmd_uninstall_service() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let path = Path::new("/etc/systemd/system/tetron.service");
        if path.exists() {
            run_cmd("systemctl", &["disable", "--now", "tetron"]);
            std::fs::remove_file(path)?;
            run_cmd("systemctl", &["daemon-reload"]);
            println!("Removed systemd service.");
        } else {
            println!("Service not installed.");
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let path = Path::new("/Library/LaunchDaemons/com.tetron.vpn.plist");
        if path.exists() {
            run_cmd("launchctl", &["unload", "-w", &path.to_string_lossy()]);
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
