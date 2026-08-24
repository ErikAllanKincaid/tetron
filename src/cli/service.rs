//! CLI service-management handlers: resume, install, start/stop/restart,
//! uninstall, operator, plus small process/daemon-reachability helpers.

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

/// Whether systemd is actually running as PID 1's init system, not just
/// whether a `systemctl` binary happens to exist on `PATH` (some minimal/
/// container environments stub or partially install one without systemd
/// genuinely running). `/run/systemd/system` is the canonical, widely-used
/// check for this (systemd itself creates it only when it is actually
/// init) -- checking for the directory's existence rather than shelling
/// out to `systemctl` avoids a confusing raw "command not found" on a
/// system (Alpine/OpenRC, Void/runit, Devuan/Artix/sysvinit, Gentoo with
/// OpenRC, ...) that has neither.
#[cfg(target_os = "linux")]
pub(crate) fn systemd_available() -> bool {
    Path::new("/run/systemd/system").exists()
}

/// Print a clear, actionable error and exit non-zero when systemd is
/// required but not present, instead of letting a bare `systemctl` call
/// fail with a raw "command not found." The daemon itself has no systemd
/// dependency (`tetron daemon` runs standalone under any supervisor) --
/// only these convenience commands do, so the message points at that
/// documented fallback rather than implying tetron cannot run at all.
#[cfg(target_os = "linux")]
pub(crate) fn require_systemd() {
    if !systemd_available() {
        eprintln!(
            "this command manages the system service via systemd, which this system\n\
             does not have (checked for /run/systemd/system). Run the daemon directly\n\
             under your own init system instead: `tetron daemon` runs standalone with\n\
             no systemd dependency of its own -- see contrib/ for a reference unit for\n\
             at least one alternative init system, and the README's \"Non-systemd Linux\"\n\
             section for the full explanation."
        );
        std::process::exit(1);
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

/// Path overrides a user can pass to `tetron install` to relocate config,
/// logs, and/or the IPC socket. Each corresponds to an `Environment=` entry
/// in the service unit (PORTABILITY-004, matching the tetron-webui `--port`
/// pattern). `None` means "use the compiled default" — the unit gets no
/// `Environment=` line for that var.
pub(crate) struct PathOverrides {
    pub config_dir: Option<String>,
    pub log_dir: Option<String>,
    pub socket_path: Option<String>,
}

/// Write the system service unit/plist, substituting the path of the binary
/// currently running so the service execs the same binary the user invoked
/// (rather than a hardcoded /usr/local/bin/tetron), and injecting any
/// path-override env vars the user requested. Idempotent — safe to call on
/// every `tetron install`, keeping the exec path fresh if the binary moves.
#[allow(unused_variables)]
pub(crate) fn ensure_service_installed(overrides: &PathOverrides) -> Result<()> {
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
        let mut service =
            include_str!("../../contrib/tetron.service").replace("/usr/local/bin/tetron", &exe);
        // Inject path-override env vars or remove the line if unset so a
        // user who passes no flags gets the exact same unit as before.
        fn inject_env(service: &mut String, env_var: &str, value: &Option<String>) {
            let placeholder = format!("__TETRON_{env_var}__");
            match value {
                Some(v) => *service = service.replace(&placeholder, v),
                None => {
                    let line = format!("Environment=TETRON_{env_var}={placeholder}\n");
                    *service = service.replace(&line, "");
                }
            }
        }
        inject_env(&mut service, "CONFIG_DIR", &overrides.config_dir);
        inject_env(&mut service, "LOG_DIR", &overrides.log_dir);
        inject_env(&mut service, "SOCKET_PATH", &overrides.socket_path);
        println!("installing systemd service 'tetron' -> {}", path.display());
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
        let mut plist = include_str!("../../contrib/com.tetron.vpn.plist")
            .replace("/usr/local/bin/tetron", &exe);
        // Inject path-override env vars or remove the XML block if unset.
        fn inject_plist_env(plist: &mut String, env_var: &str, value: &Option<String>) {
            let placeholder = format!("__TETRON_{env_var}__");
            match value {
                Some(v) => *plist = plist.replace(&placeholder, v),
                None => {
                    let block = format!(
                        "        <key>TETRON_{env_var}</key>\n        <string>{placeholder}</string>\n"
                    );
                    *plist = plist.replace(&block, "");
                }
            }
        }
        inject_plist_env(&mut plist, "CONFIG_DIR", &overrides.config_dir);
        inject_plist_env(&mut plist, "LOG_DIR", &overrides.log_dir);
        inject_plist_env(&mut plist, "SOCKET_PATH", &overrides.socket_path);
        println!(
            "installing launchd job 'com.tetron.vpn' -> {}",
            path.display()
        );
        std::fs::write(path, plist)
            .with_context(|| format!("failed to write {}", path.display()))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        anyhow::bail!("system service not supported on this platform");
    }
}

/// `tetron resume`: activate the VPN's data plane.
///
/// A stable, single-meaning operation (CLI-VOCAB-004): an unprivileged IPC
/// call asking the already-running daemon to bring the TUN up and reconnect
/// networks. Unlike the old `up`, this never silently installs or starts the
/// system service -- if no daemon is reachable, it errors the same way
/// regardless of caller privilege and points at the actual bootstrap command.
pub(crate) async fn cmd_resume(hostname: Option<String>, network: Option<String>) -> Result<()> {
    let Ok(mut stream) = ipc::connect().await else {
        eprintln!("tetron service is not running. Install and start it with: sudo tetron install");
        std::process::exit(1);
    };
    ipc::send(&mut stream, ipc::IpcMessage::Resume { hostname, network }).await?;
    match ipc::recv(&mut stream).await? {
        ipc::IpcMessage::Ok { message } => println!("{message}"),
        ipc::IpcMessage::Error { message } => {
            print_error("error", &message, None);
            std::process::exit(1);
        }
        other => eprintln!("Unexpected response: {other:?}"),
    }
    Ok(())
}

/// Install/refresh the system service and (re)start it. Requires root.
///
/// Starting the service is fire-and-forget at the OS level, so we then wait for
/// the daemon to actually accept an IPC connection before declaring success. If
/// it never comes up (e.g. it crashed on a port/route conflict with another
/// VPN), we surface the tail of its log so the user knows what went wrong
/// instead of seeing a cheerful "started" followed by a dead `tetron status`.
pub(crate) async fn install_and_start_service(
    hostname: Option<String>,
    overrides: &PathOverrides,
) -> Result<()> {
    ensure_service_installed(overrides)?;

    #[cfg(target_os = "linux")]
    {
        println!("enabling and starting systemd service 'tetron' (systemctl enable/restart)");
        run_cmd("systemctl", &["enable", "tetron"]);
        run_cmd("systemctl", &["restart", "tetron"]);
    }

    #[cfg(target_os = "macos")]
    {
        let path = "/Library/LaunchDaemons/com.tetron.vpn.plist";
        // Tear down any previously loaded job (e.g. one pointing at a stale
        // binary path) before loading the freshly written plist.
        run_cmd_quiet("launchctl", &["unload", path]);
        println!("loading launchd job 'com.tetron.vpn' -> {path}");
        run_cmd("launchctl", &["load", "-w", path]);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("system service not supported on this platform");
    }

    // Propagate path overrides into this process's environment so the IPC
    // client (`wait_for_daemon` -> `ipc::connect()` -> `socket_path()`) can
    // find the daemon on the custom socket. Without this, the daemon listens
    // on the custom path (picked up from its own `Environment=` in the unit)
    // but the installer probes the default path and times out.
    if let Some(ref p) = overrides.socket_path {
        unsafe { std::env::set_var("TETRON_SOCKET_PATH", p) };
    }

    // Wait for the freshly started daemon to accept IPC, then activate the VPN.
    eprintln!("waiting for daemon…");
    let daemon = wait_for_daemon(DAEMON_REACHABLE_TIMEOUT).await;
    match daemon {
        Some(mut stream) => {
            ipc::send(
                &mut stream,
                ipc::IpcMessage::Resume {
                    hostname,
                    network: None,
                },
            )
            .await?;
            match ipc::recv(&mut stream).await? {
                ipc::IpcMessage::Ok { message } => println!("tetron service started. {message}"),
                ipc::IpcMessage::Error { message } => {
                    print_error("error", &message, None);
                    std::process::exit(1);
                }
                other => eprintln!("Unexpected response: {other:?}"),
            }
            // We're root here (installing the service). Grant the invoking user
            // operator access so they can run `tetron` without sudo from now on,
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
/// (`$SUDO_USER`) operator access so subsequent `tetron` commands work without
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

/// Full version string: the crate version plus the git short SHA stamped in by
/// `build.rs` (e.g. `0.1.0 (abc12345)`).
pub(crate) const FULL_VERSION: &str =
    concat!(env!("CARGO_PKG_VERSION"), " (", env!("TETRON_GIT_SHA"), ")");

/// `tetron install`: install the system service if needed (or refresh an existing
/// install), then start it and verify the daemon comes up (INSTALL-VERSION-001). Requires root.
///
/// Optional `--config-dir`, `--log-dir`, `--socket-path` flags inject the
/// corresponding `Environment=` lines into the service unit so the daemon
/// uses nonstandard paths (PORTABILITY-004).
pub(crate) async fn cmd_install(
    config_dir: Option<String>,
    log_dir: Option<String>,
    socket_path: Option<String>,
) -> Result<()> {
    require_root()?;
    #[cfg(target_os = "linux")]
    require_systemd();
    println!("installing tetron {FULL_VERSION}");
    let overrides = PathOverrides {
        config_dir,
        log_dir,
        socket_path,
    };
    install_and_start_service(None, &overrides).await
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
/// `tetron restart`; mirrors the `install` diagnostics.
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
    #[cfg(target_os = "linux")]
    require_systemd();
    if !service_unit_exists() {
        eprintln!("tetron service is not installed. Run: sudo tetron install");
        std::process::exit(1);
    }
    restart_service_and_wait().await
}

/// `tetron stop`: stop the installed system service so the daemon exits and all
/// peer connections close cleanly (a clean offline, distinct from `tetron
/// standby`). Does not disable or uninstall the unit. Requires root.
#[allow(unreachable_code)]
pub(crate) async fn cmd_stop() -> Result<()> {
    require_root()?;
    #[cfg(target_os = "linux")]
    require_systemd();
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
    #[cfg(target_os = "linux")]
    require_systemd();
    if !service_unit_exists() {
        eprintln!("tetron service is not installed. Run: sudo tetron install");
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
    // SELFCAPTURE-ROUTE-001: torn down only here, not on ordinary `tetron
    // stop`/restart -- mirrors TUN devices, which are likewise only removed
    // on actual network leave/nuke, never on ordinary stop/start.
    tetron::selfcapture::teardown();

    #[cfg(target_os = "linux")]
    {
        let path = Path::new("/etc/systemd/system/tetron.service");
        if path.exists() {
            require_systemd();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_version_non_empty() {
        assert!(!FULL_VERSION.is_empty());
        assert!(FULL_VERSION.contains(env!("CARGO_PKG_VERSION")));
    }
}
