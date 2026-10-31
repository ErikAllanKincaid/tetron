//! CLI service-management handlers: resume, install, start/stop/restart,
//! uninstall, operator, plus small process/daemon-reachability helpers.

use crate::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Create the `tetron` system group if it doesn't already exist (Linux).
/// Best-effort: the daemon's config writer falls back to `root:root` ownership
/// when the group is missing, so a failure here only loosens the group-read
/// posture, never breaks startup.
#[cfg(target_os = "linux")]
pub(crate) fn ensure_tetron_group() {
    // `getent group tetron` exits 0 if the group exists.
    // PACKAGING-014: only the exit status matters; getent's line is discarded.
    let exists = Command::new("getent")
        .args(["group", "tetron"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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

/// Parse `dpkg -S` output into the owning package name (PACKAGING-001).
///
/// Accepts `pkg: /path`, `pkg:arch: /path` and `a, b: /path` (first owner
/// wins); ignores `diversion by ...` lines and anything that is not a package
/// name followed by an absolute path, such as dpkg's "no path found" message.
pub(crate) fn parse_dpkg_search(out: &str) -> Option<String> {
    out.lines()
        .filter(|l| !l.starts_with("diversion by "))
        .find_map(|l| {
            let (owners, path) = l.split_once(": ")?;
            if !path.starts_with('/') {
                return None;
            }
            let name = owners.split(',').next()?.trim().split(':').next()?;
            let valid = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
                && name.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.')
                });
            valid.then(|| name.to_string())
        })
}

/// Name of the package that owns `path`, asking through the injected `run`
/// (which returns `dpkg -S` stdout on success). The path is stripped of the
/// ` (deleted)` marker and symlink-resolved first. Never errors: no owner, no
/// dpkg, or a failing command all give `None` (PACKAGING-001).
pub(crate) fn package_owner_with(
    path: &Path,
    run: &dyn Fn(&Path) -> Option<String>,
) -> Option<String> {
    let stripped = PathBuf::from(strip_deleted_suffix(&path.to_string_lossy()));
    let resolved = stripped.canonicalize().unwrap_or(stripped);
    parse_dpkg_search(&run(&resolved)?)
}

/// `dpkg -S <path>` stdout when dpkg exists and succeeds.
fn dpkg_search(path: &Path) -> Option<String> {
    let out = Command::new("dpkg")
        .arg("-S")
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The package that owns `path` on a Debian-family host, else `None`
/// (PACKAGING-001). Used by PACKAGING-004 and PACKAGING-005.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn package_owner(path: &Path) -> Option<String> {
    package_owner_with(path, &dpkg_search)
}

/// Path overrides a user can pass to `tetron install` to relocate config,
/// logs, and/or the IPC socket. Each corresponds to an `Environment=` entry
/// in the service unit (PORTABILITY-004, matching the tetron-webui `--port`
/// pattern). `None` means "use the compiled default" — the unit gets no
/// `Environment=` line for that var.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PathOverrides {
    pub config_dir: Option<String>,
    pub log_dir: Option<String>,
    pub socket_path: Option<String>,
}

/// What `tetron install` does about the systemd unit on Linux (PACKAGING-004).
#[derive(Debug, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum UnitPlan {
    /// Write `/etc/systemd/system/tetron.service` with the running exe path.
    WriteUnit,
    /// A package owns the binary and ships a vendor unit: leave the unit alone
    /// and apply any path overrides as a systemd drop-in.
    UsePackaged {
        package: String,
        dropin: Option<String>,
    },
}

/// Decide how to install the unit: leave a packaged one alone when the
/// executable is package-owned and a vendor unit exists, otherwise write the
/// unit as before (PACKAGING-004).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn plan_linux_unit(
    owner: Option<String>,
    vendor_unit_exists: bool,
    overrides: &PathOverrides,
) -> UnitPlan {
    match owner {
        Some(package) if vendor_unit_exists => UnitPlan::UsePackaged {
            package,
            dropin: dropin_text(overrides),
        },
        _ => UnitPlan::WriteUnit,
    }
}

/// systemd drop-in carrying only the path overrides that were given, in the
/// order config, log, socket; `None` when none were given (PACKAGING-004).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn dropin_text(overrides: &PathOverrides) -> Option<String> {
    let lines: String = [
        ("CONFIG_DIR", &overrides.config_dir),
        ("LOG_DIR", &overrides.log_dir),
        ("SOCKET_PATH", &overrides.socket_path),
    ]
    .iter()
    .filter_map(|(key, value)| {
        value
            .as_ref()
            .map(|v| format!("Environment=TETRON_{key}={v}\n"))
    })
    .collect();
    (!lines.is_empty()).then(|| format!("[Service]\n{lines}"))
}

/// Write the path-override drop-in `10-paths.conf` into `dir`, creating the
/// directory; replaces any earlier content (PACKAGING-004).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn write_dropin(dir: &Path, text: &str) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let file = dir.join("10-paths.conf");
    std::fs::write(&file, text).with_context(|| format!("failed to write {}", file.display()))
}

/// One-line notice when a unit that `tetron install` wrote earlier still
/// exists: systemd lets it override the packaged unit (PACKAGING-004).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn legacy_unit_notice(etc_unit_exists: bool) -> Option<String> {
    etc_unit_exists.then(|| {
        "note: /etc/systemd/system/tetron.service takes precedence over the packaged unit; \
         remove it and run `systemctl daemon-reload` to use the packaged one"
            .to_string()
    })
}

/// The line `tetron install` prints once the daemon answered the resume
/// request (PACKAGING-015).
pub(crate) fn started_line(reply: &str) -> String {
    let reply = reply.trim();
    if reply.is_empty() {
        "tetron service started.".to_string()
    } else {
        format!("tetron service started; VPN: {reply}")
    }
}

/// What uninstall and package removal keep, and how to delete it by hand
/// (PACKAGING-013, purge policy).
pub(crate) fn kept_data_notice(config_dir: &Path, log_dir: &Path) -> String {
    format!(
        "Kept: the node identity and settings in {c} and the logs in {l} \
         (removing tetron never deletes them; without the identity this node \
         drops out of its networks). To delete them: \
         sudo rm -rf {c} {l} && sudo groupdel tetron",
        c = config_dir.display(),
        l = log_dir.display()
    )
}

/// Description line of the unit template `tetron install` writes.
const UNIT_TEMPLATE_DESCRIPTION: &str = "Description=tetron P2P mesh VPN";

/// Path overrides carried by an `/etc` unit that `tetron install` wrote, or
/// `None` when the unit is not one of ours: its description is not the
/// template's, or its `ExecStart` does not run a `tetron` binary with the
/// `daemon` argument (PACKAGING-016).
pub(crate) fn legacy_unit_overrides(text: &str) -> Option<PathOverrides> {
    if !text.lines().any(|l| l.trim() == UNIT_TEMPLATE_DESCRIPTION) {
        return None;
    }
    let exec = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))?;
    let mut words = exec.split_whitespace();
    let program = words.next()?;
    let is_tetron = Path::new(program)
        .file_name()
        .is_some_and(|n| n == "tetron");
    if !is_tetron || words.next() != Some("daemon") {
        return None;
    }
    let env = |key: &str| {
        let prefix = format!("Environment=TETRON_{key}=");
        text.lines()
            .find_map(|l| l.trim().strip_prefix(prefix.as_str()).map(str::to_string))
            .filter(|v| !v.is_empty() && !v.starts_with("__TETRON_"))
    };
    Some(PathOverrides {
        config_dir: env("CONFIG_DIR"),
        log_dir: env("LOG_DIR"),
        socket_path: env("SOCKET_PATH"),
    })
}

/// Flags given on this run win; the legacy unit fills the gaps (PACKAGING-016).
pub(crate) fn merge_overrides(flags: &PathOverrides, legacy: &PathOverrides) -> PathOverrides {
    PathOverrides {
        config_dir: flags
            .config_dir
            .clone()
            .or_else(|| legacy.config_dir.clone()),
        log_dir: flags.log_dir.clone().or_else(|| legacy.log_dir.clone()),
        socket_path: flags
            .socket_path
            .clone()
            .or_else(|| legacy.socket_path.clone()),
    }
}

/// Move `file` into `backup_dir` as `<name>.<stamp>`, creating the directory.
/// Copies then removes, so it works across filesystems (PACKAGING-016).
pub(crate) fn move_to_backup(file: &Path, backup_dir: &Path, stamp: u64) -> Result<PathBuf> {
    std::fs::create_dir_all(backup_dir)
        .with_context(|| format!("failed to create {}", backup_dir.display()))?;
    let name = file
        .file_name()
        .context("unit path has no file name")?
        .to_string_lossy();
    let dest = backup_dir.join(format!("{name}.{stamp}"));
    std::fs::copy(file, &dest)
        .with_context(|| format!("failed to copy {} to {}", file.display(), dest.display()))?;
    std::fs::remove_file(file).with_context(|| format!("failed to remove {}", file.display()))?;
    Ok(dest)
}

/// Ask at a terminal whether to move the legacy `/etc` unit aside. Default
/// yes; without a terminal the answer is no (PACKAGING-016).
#[cfg(target_os = "linux")]
fn confirm_move_legacy_unit(path: &Path) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!(
        "{} (written by an earlier `tetron install`) overrides the packaged unit. \
         Move it to /var/backups/tetron/ and use the packaged unit? [Y/n] ",
        path.display()
    );
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    !matches!(answer.trim(), "n" | "N" | "no" | "No")
}

/// Whether a package-installed unit exists in a vendor directory.
#[cfg(target_os = "linux")]
fn vendor_unit_exists() -> bool {
    vendor_unit_path().is_some()
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
        let vendor = vendor_unit_exists();
        let owner = if vendor {
            package_owner(Path::new(&exe))
        } else {
            None
        };
        if let UnitPlan::UsePackaged { package, dropin } = plan_linux_unit(owner, vendor, overrides)
        {
            println!(
                "tetron is managed by package '{package}'; leaving the packaged unit in place"
            );
            let mut dropin = dropin;
            // PACKAGING-016: move a unit an earlier `tetron install` wrote out of
            // the way, keeping its path overrides, when the user agrees.
            let legacy = std::fs::read_to_string(path)
                .ok()
                .and_then(|t| legacy_unit_overrides(&t));
            match legacy {
                Some(old) if confirm_move_legacy_unit(path) => {
                    let merged = merge_overrides(overrides, &old);
                    dropin = dropin_text(&merged);
                    run_cmd("systemctl", &["disable", "tetron"]);
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let moved = move_to_backup(path, Path::new("/var/backups/tetron"), stamp)?;
                    println!("moved {} -> {}", path.display(), moved.display());
                    run_cmd("systemctl", &["daemon-reload"]);
                }
                _ => {
                    if let Some(note) = legacy_unit_notice(path.exists()) {
                        println!("{note}");
                    }
                }
            }
            if let Some(text) = dropin {
                let dir = Path::new("/etc/systemd/system/tetron.service.d");
                write_dropin(dir, &text)?;
                println!(
                    "wrote path overrides -> {}",
                    dir.join("10-paths.conf").display()
                );
                run_cmd("systemctl", &["daemon-reload"]);
            }
            return Ok(());
        }
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
                ipc::IpcMessage::Ok { message } => println!("{}", started_line(&message)),
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

/// Directories systemd reads service units from, in the places this tool or a
/// package manager puts tetron's unit: the admin directory that
/// `tetron install` writes to, and the vendor directories a package installs
/// into (PACKAGING-003).
#[cfg(target_os = "linux")]
const UNIT_DIRS: [&str; 3] = [
    "/etc/systemd/system",
    "/usr/lib/systemd/system",
    "/lib/systemd/system",
];

/// Whether `tetron.service` exists in any of `dirs` (PACKAGING-003).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn unit_exists_in(dirs: &[&Path]) -> bool {
    dirs.iter().any(|d| d.join("tetron.service").exists())
}

/// Whether the system service unit/plist is installed on this host. On Linux
/// a unit installed by a package under `/usr/lib` or `/lib` counts, not only
/// the `/etc` unit that `tetron install` writes (PACKAGING-003).
pub(crate) fn service_unit_exists() -> bool {
    #[cfg(target_os = "linux")]
    {
        let dirs = UNIT_DIRS.map(Path::new);
        return unit_exists_in(&dirs);
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

/// What `tetron uninstall` does on Linux (PACKAGING-005).
#[derive(Debug, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) struct UninstallPlan {
    /// Any tetron unit exists, so the service is disabled and stopped.
    pub installed: bool,
    /// Delete `/etc/systemd/system/tetron.service`, which `tetron install` wrote.
    pub remove_etc_unit: bool,
    /// Said when a package-installed unit remains: it is never deleted here.
    pub hint: Option<String>,
}

/// Decide the uninstall steps from which units exist and who owns the vendor
/// unit. A vendor unit under `/usr/lib` or `/lib` is never deleted
/// (PACKAGING-005).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn plan_uninstall(
    etc_unit_exists: bool,
    vendor_unit_exists: bool,
    vendor_owner: Option<String>,
) -> UninstallPlan {
    let hint = vendor_unit_exists.then(|| match vendor_owner {
        Some(pkg) => format!(
            "The service unit is installed by package '{pkg}' and was not deleted; \
             remove it with: sudo apt remove {pkg}"
        ),
        None => "The service unit is installed by a package and was not deleted; \
                 remove it with your package manager"
            .to_string(),
    });
    UninstallPlan {
        installed: etc_unit_exists || vendor_unit_exists,
        remove_etc_unit: etc_unit_exists,
        hint,
    }
}

/// Path of the first package-installed unit, if any.
#[cfg(target_os = "linux")]
fn vendor_unit_path() -> Option<PathBuf> {
    UNIT_DIRS[1..]
        .iter()
        .map(|d| Path::new(d).join("tetron.service"))
        .find(|p| p.exists())
}

/// Print what uninstall keeps, for the directories actually in use
/// (PACKAGING-013).
/// `TETRON_CONFIG_DIR` and `TETRON_LOG_DIR` from systemd's `Environment=`
/// property (`systemctl show -p Environment --value`), so uninstall names the
/// directories the service actually used (PACKAGING-013).
pub(crate) fn service_env_dirs(env: &str) -> (Option<String>, Option<String>) {
    let get = |key: &str| {
        env.split_whitespace()
            .find_map(|kv| kv.strip_prefix(key).map(str::to_string))
            .filter(|v| !v.is_empty())
    };
    (get("TETRON_CONFIG_DIR="), get("TETRON_LOG_DIR="))
}

/// Print what uninstall keeps. `service_env` is the unit's `Environment=`
/// value when known; it wins over this process's defaults (PACKAGING-013).
fn print_kept_data_notice(service_env: Option<&str>) {
    let (config, log) = service_env.map(service_env_dirs).unwrap_or((None, None));
    let config = match config {
        Some(c) => PathBuf::from(c),
        None => match tetron::config::config_dir() {
            Ok(c) => c,
            Err(_) => return,
        },
    };
    let log = log
        .map(PathBuf::from)
        .unwrap_or_else(tetron::logdir::log_dir);
    println!("{}", kept_data_notice(&config, &log));
}

pub(crate) fn cmd_uninstall_service() -> Result<()> {
    // SELFCAPTURE-ROUTE-001: torn down only here, not on ordinary `tetron
    // stop`/restart -- mirrors TUN devices, which are likewise only removed
    // on actual network leave/nuke, never on ordinary stop/start.
    tetron::selfcapture::teardown();

    #[cfg(target_os = "linux")]
    {
        let path = Path::new("/etc/systemd/system/tetron.service");
        let vendor = vendor_unit_path();
        let owner = vendor.as_deref().and_then(package_owner);
        let plan = plan_uninstall(path.exists(), vendor.is_some(), owner);
        if !plan.installed {
            println!("Service not installed.");
            return Ok(());
        }
        require_systemd();
        // Read the directories the service uses before it is disabled.
        let service_env = Command::new("systemctl")
            .args(["show", "-p", "Environment", "--value", "tetron"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        run_cmd("systemctl", &["disable", "--now", "tetron"]);
        if plan.remove_etc_unit {
            std::fs::remove_file(path)?;
            run_cmd("systemctl", &["daemon-reload"]);
            println!("Removed systemd service.");
        }
        if let Some(hint) = plan.hint {
            println!("{hint}");
        }
        print_kept_data_notice(service_env.as_deref());
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let path = Path::new("/Library/LaunchDaemons/com.tetron.vpn.plist");
        if path.exists() {
            run_cmd("launchctl", &["unload", "-w", &path.to_string_lossy()]);
            std::fs::remove_file(path)?;
            println!("Removed launchd daemon.");
            print_kept_data_notice(None);
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

    // PACKAGING-001: package owner detection.

    #[test]
    fn parse_dpkg_search_plain_owner() {
        assert_eq!(
            parse_dpkg_search("tetron: /usr/bin/tetron\n"),
            Some("tetron".to_string())
        );
    }

    #[test]
    fn parse_dpkg_search_strips_architecture_qualifier() {
        assert_eq!(
            parse_dpkg_search("tetron:amd64: /usr/bin/tetron\n"),
            Some("tetron".to_string())
        );
    }

    #[test]
    fn parse_dpkg_search_takes_first_of_several_owners() {
        assert_eq!(
            parse_dpkg_search("alpha, beta: /usr/bin/tool\n"),
            Some("alpha".to_string())
        );
    }

    #[test]
    fn parse_dpkg_search_skips_diversion_lines() {
        let out = "diversion by other from: /usr/bin/tetron\n\
                   diversion by other to: /usr/bin/tetron.distrib\n\
                   tetron: /usr/bin/tetron\n";
        assert_eq!(parse_dpkg_search(out), Some("tetron".to_string()));
    }

    #[test]
    fn parse_dpkg_search_unowned_or_garbage_is_none() {
        assert_eq!(parse_dpkg_search(""), None);
        assert_eq!(
            parse_dpkg_search("dpkg-query: no path found matching pattern /usr/local/bin/tetron\n"),
            None
        );
        assert_eq!(parse_dpkg_search("\n\n"), None);
        assert_eq!(parse_dpkg_search("Not A Package: /x\n"), None);
    }

    #[test]
    fn package_owner_with_strips_deleted_marker_before_asking() {
        let seen = std::cell::RefCell::new(None);
        let run = |p: &Path| {
            *seen.borrow_mut() = Some(p.to_path_buf());
            Some("tetron: /usr/bin/tetron\n".to_string())
        };
        let got = package_owner_with(Path::new("/nonexistent-dir/tetron (deleted)"), &run);
        assert_eq!(got, Some("tetron".to_string()));
        assert_eq!(
            seen.borrow().as_deref(),
            Some(Path::new("/nonexistent-dir/tetron"))
        );
    }

    #[test]
    fn package_owner_with_resolves_symlinks_before_asking() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-tetron");
        std::fs::write(&real, b"x").unwrap();
        let link = dir.path().join("link-tetron");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let seen = std::cell::RefCell::new(None);
        let run = |p: &Path| {
            *seen.borrow_mut() = Some(p.to_path_buf());
            None
        };
        assert_eq!(package_owner_with(&link, &run), None);
        assert_eq!(
            seen.borrow().as_deref(),
            Some(real.canonicalize().unwrap().as_path())
        );
    }

    // PACKAGING-003: vendor service unit detection.

    #[test]
    fn unit_exists_in_finds_unit_in_any_directory() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert!(!unit_exists_in(&[a.path(), b.path()]));
        assert!(!unit_exists_in(&[]));
        std::fs::write(b.path().join("tetron.service"), b"[Unit]\n").unwrap();
        assert!(unit_exists_in(&[a.path(), b.path()]));
        assert!(unit_exists_in(&[b.path()]));
        assert!(!unit_exists_in(&[a.path()]));
    }

    #[test]
    fn unit_exists_in_ignores_other_unit_names() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("tetron-webui.service"), b"[Unit]\n").unwrap();
        assert!(!unit_exists_in(&[a.path()]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unit_dirs_cover_etc_and_vendor_locations() {
        assert_eq!(
            UNIT_DIRS,
            [
                "/etc/systemd/system",
                "/usr/lib/systemd/system",
                "/lib/systemd/system"
            ]
        );
    }

    // PACKAGING-002: the packaged unit cannot drift from the installed one.

    #[test]
    fn packaged_unit_differs_from_installed_unit_only_in_exec_path_and_placeholders() {
        let installed = include_str!("../../contrib/tetron.service");
        let packaged = include_str!("../../contrib/tetron.packaged.service");
        let expected: Vec<String> = installed
            .lines()
            .filter(|l| !l.contains("__TETRON_"))
            .map(|l| l.replace("/usr/local/bin/tetron", "/usr/bin/tetron"))
            .collect();
        let got: Vec<String> = packaged.lines().map(str::to_string).collect();
        assert_eq!(expected, got);
        assert!(!packaged.contains("__TETRON_"));
        assert!(packaged.contains("ExecStart=/usr/bin/tetron daemon"));
    }

    // PACKAGING-004: install is package aware.

    fn overrides(c: Option<&str>, l: Option<&str>, s: Option<&str>) -> PathOverrides {
        PathOverrides {
            config_dir: c.map(str::to_string),
            log_dir: l.map(str::to_string),
            socket_path: s.map(str::to_string),
        }
    }

    #[test]
    fn plan_writes_unit_when_not_package_owned() {
        let plan = plan_linux_unit(None, true, &overrides(None, None, None));
        assert_eq!(plan, UnitPlan::WriteUnit);
    }

    #[test]
    fn plan_writes_unit_when_owned_but_no_vendor_unit() {
        let plan = plan_linux_unit(Some("tetron".into()), false, &overrides(None, None, None));
        assert_eq!(plan, UnitPlan::WriteUnit);
    }

    #[test]
    fn plan_uses_packaged_unit_without_dropin_when_no_overrides() {
        let plan = plan_linux_unit(Some("tetron".into()), true, &overrides(None, None, None));
        assert_eq!(
            plan,
            UnitPlan::UsePackaged {
                package: "tetron".into(),
                dropin: None
            }
        );
    }

    #[test]
    fn plan_dropin_has_exactly_the_given_overrides() {
        let plan = plan_linux_unit(
            Some("tetron".into()),
            true,
            &overrides(Some("/srv/tetron"), None, Some("/run/t.sock")),
        );
        assert_eq!(
            plan,
            UnitPlan::UsePackaged {
                package: "tetron".into(),
                dropin: Some(
                    "[Service]\nEnvironment=TETRON_CONFIG_DIR=/srv/tetron\n\
                     Environment=TETRON_SOCKET_PATH=/run/t.sock\n"
                        .into()
                )
            }
        );
    }

    #[test]
    fn dropin_text_orders_config_log_socket() {
        let t = dropin_text(&overrides(Some("/c"), Some("/l"), Some("/s"))).unwrap();
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(
            lines,
            [
                "[Service]",
                "Environment=TETRON_CONFIG_DIR=/c",
                "Environment=TETRON_LOG_DIR=/l",
                "Environment=TETRON_SOCKET_PATH=/s"
            ]
        );
        assert_eq!(dropin_text(&overrides(None, None, None)), None);
    }

    #[test]
    fn write_dropin_creates_directory_and_file() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("tetron.service.d");
        write_dropin(&dir, "[Service]\nEnvironment=X=1\n").unwrap();
        let got = std::fs::read_to_string(dir.join("10-paths.conf")).unwrap();
        assert_eq!(got, "[Service]\nEnvironment=X=1\n");
        // Idempotent: a second write replaces the content.
        write_dropin(&dir, "[Service]\nEnvironment=X=2\n").unwrap();
        let got = std::fs::read_to_string(dir.join("10-paths.conf")).unwrap();
        assert!(got.contains("X=2") && !got.contains("X=1"));
    }

    #[test]
    fn legacy_unit_notice_only_when_etc_unit_exists() {
        assert_eq!(legacy_unit_notice(false), None);
        let n = legacy_unit_notice(true).unwrap();
        assert!(n.contains("/etc/systemd/system/tetron.service"));
        assert!(n.contains("takes precedence"));
    }

    // PACKAGING-005: uninstall is package aware.

    #[test]
    fn uninstall_plan_etc_unit_only_matches_previous_behavior() {
        let plan = plan_uninstall(true, false, None);
        assert_eq!(
            plan,
            UninstallPlan {
                installed: true,
                remove_etc_unit: true,
                hint: None
            }
        );
    }

    #[test]
    fn uninstall_plan_vendor_unit_only_disables_and_points_at_apt() {
        let plan = plan_uninstall(false, true, Some("tetron".into()));
        assert!(plan.installed);
        assert!(!plan.remove_etc_unit);
        let hint = plan.hint.unwrap();
        assert!(hint.contains("sudo apt remove tetron"));
        assert!(hint.contains("not deleted"));
    }

    #[test]
    fn uninstall_plan_vendor_unit_unknown_owner_names_the_package_manager() {
        let plan = plan_uninstall(false, true, None);
        let hint = plan.hint.unwrap();
        assert!(hint.contains("package manager"));
        assert!(!hint.contains("apt remove"));
    }

    #[test]
    fn uninstall_plan_both_units_removes_only_the_etc_one() {
        let plan = plan_uninstall(true, true, Some("tetron".into()));
        assert!(plan.installed);
        assert!(plan.remove_etc_unit);
        assert!(plan.hint.unwrap().contains("sudo apt remove tetron"));
    }

    #[test]
    fn uninstall_plan_nothing_installed() {
        let plan = plan_uninstall(false, false, None);
        assert_eq!(
            plan,
            UninstallPlan {
                installed: false,
                remove_etc_unit: false,
                hint: None
            }
        );
    }

    #[test]
    fn package_owner_with_none_when_runner_has_no_output() {
        let run = |_: &Path| None;
        assert_eq!(
            package_owner_with(Path::new("/usr/local/bin/tetron"), &run),
            None
        );
    }

    // PACKAGING-013: uninstall says what it keeps.
    #[test]
    fn kept_data_notice_names_dirs_and_removal_command() {
        let n = kept_data_notice(Path::new("/etc/tetron"), Path::new("/var/log/tetron"));
        assert!(n.contains("/etc/tetron"), "{n}");
        assert!(n.contains("/var/log/tetron"), "{n}");
        assert!(n.contains("identity"), "{n}");
        assert!(
            n.contains("sudo rm -rf /etc/tetron /var/log/tetron && sudo groupdel tetron"),
            "{n}"
        );
    }

    #[test]
    fn kept_data_notice_uses_custom_dirs() {
        let n = kept_data_notice(Path::new("/srv/t"), Path::new("/srv/l"));
        assert!(n.contains("sudo rm -rf /srv/t /srv/l"), "{n}");
    }

    #[test]
    fn service_env_dirs_reads_overrides() {
        assert_eq!(
            service_env_dirs("TETRON_LOG_DIR=/var/log/tetron-alt TETRON_SOCKET_PATH=/run/t.sock\n"),
            (None, Some("/var/log/tetron-alt".to_string()))
        );
        assert_eq!(
            service_env_dirs("TETRON_CONFIG_DIR=/srv/t"),
            (Some("/srv/t".to_string()), None)
        );
        assert_eq!(service_env_dirs(""), (None, None));
    }

    // PACKAGING-015: the started line reads correctly.
    #[test]
    fn started_line_with_reply() {
        assert_eq!(
            started_line("already active"),
            "tetron service started; VPN: already active"
        );
    }

    #[test]
    fn started_line_without_reply() {
        assert_eq!(started_line(""), "tetron service started.");
        assert_eq!(started_line("  "), "tetron service started.");
    }

    // PACKAGING-016: the legacy /etc unit is recognised and migrated.
    #[test]
    fn legacy_unit_from_plain_template_has_no_overrides() {
        let text = include_str!("../../contrib/tetron.service")
            .replace("/usr/local/bin/tetron", "/usr/local/bin/tetron")
            .replace("Environment=TETRON_CONFIG_DIR=__TETRON_CONFIG_DIR__\n", "")
            .replace("Environment=TETRON_LOG_DIR=__TETRON_LOG_DIR__\n", "")
            .replace(
                "Environment=TETRON_SOCKET_PATH=__TETRON_SOCKET_PATH__\n",
                "",
            );
        let got = legacy_unit_overrides(&text).expect("tetron-written unit");
        assert_eq!(got, overrides(None, None, None));
    }

    #[test]
    fn legacy_unit_overrides_are_read() {
        let text = include_str!("../../contrib/tetron.service")
            .replace("__TETRON_CONFIG_DIR__", "/srv/tetron")
            .replace("Environment=TETRON_LOG_DIR=__TETRON_LOG_DIR__\n", "")
            .replace("__TETRON_SOCKET_PATH__", "/run/t.sock");
        let got = legacy_unit_overrides(&text).expect("tetron-written unit");
        assert_eq!(
            got,
            overrides(Some("/srv/tetron"), None, Some("/run/t.sock"))
        );
    }

    #[test]
    fn foreign_unit_is_not_legacy() {
        let custom = "[Unit]\nDescription=my own tetron\n[Service]\nExecStart=/opt/tetron daemon\n";
        assert!(legacy_unit_overrides(custom).is_none());
        let other_exec = "[Unit]\nDescription=tetron P2P mesh VPN\n[Service]\nExecStart=/usr/bin/somethingelse run\n";
        assert!(legacy_unit_overrides(other_exec).is_none());
    }

    #[test]
    fn flags_win_over_legacy_overrides() {
        let flags = overrides(Some("/flag/c"), None, None);
        let legacy = overrides(Some("/old/c"), Some("/old/l"), None);
        assert_eq!(
            merge_overrides(&flags, &legacy),
            overrides(Some("/flag/c"), Some("/old/l"), None)
        );
    }

    #[test]
    fn move_to_backup_keeps_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let unit = tmp.path().join("tetron.service");
        std::fs::write(&unit, "x").unwrap();
        let backup_dir = tmp.path().join("backups/tetron");
        let moved = move_to_backup(&unit, &backup_dir, 1234).unwrap();
        assert!(!unit.exists());
        assert_eq!(moved, backup_dir.join("tetron.service.1234"));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "x");
    }
}
