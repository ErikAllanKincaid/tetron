// The daemon's modules live in the `tetron` library crate (`src/lib.rs`) so
// integration tests and benchmarks can reach them; this binary is the CLI/IPC
// client built on top.
use tetron::{
    config, daemon, invite, ipc, logdir, membership, shutdown, stats,
};

use std::sync::{Arc, atomic};

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

use membership::GroupMode;

// The CLI command handlers are split into the `cli` module (`src/cli/`) to keep
// this file to the clap definitions + dispatch. `cli` re-exports each domain
// submodule's contents, and `use cli::*` flattens them into the crate root so
// every handler resolves the others (and the shared helpers here) by name.
mod cli;
use cli::*;

/// Full version string: the crate version plus the git short SHA stamped in by
/// `build.rs` (e.g. `0.1.0 (abc12345)`). The SHA distinguishes nightly builds
/// that share a crate version, and is what a tester quotes in a bug report.
const FULL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("RAY_GIT_SHA"), ")");

#[derive(Parser)]
#[command(
    name = "torpedo",
    about = "P2P mesh VPN powered by iroh",
    version = FULL_VERSION
)]
struct Cli {
    /// Emit machine-readable JSON instead of styled text (disables color and
    /// spinners). Supported by `status`, `invite list`, `requests`, and other
    /// list commands.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

static JSON_FLAG: atomic::AtomicBool = atomic::AtomicBool::new(false);

/// Whether `--json` output mode is active (set once in `main`).
fn json_enabled() -> bool {
    JSON_FLAG.load(atomic::Ordering::Relaxed)
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Create a new network and wait for peers
    #[command(visible_alias = "new")]
    Create {
        /// Network name (a random three-word name is generated if not set)
        #[arg(long)]
        name: Option<String>,
        /// Your hostname within the network (e.g. "alice" → alice.gaming.ray). Random if not set
        #[arg(long)]
        hostname: Option<String>,
        /// Overlay IPv4 subnet in CIDR form (e.g. "10.88.0.0/16"). Override the
        /// default only if it collides with an existing local network. Defaults
        /// to 10.88.0.0/16, chosen to coexist with Tailscale's 100.64.0.0/10.
        #[arg(long)]
        subnet: Option<String>,
        /// Route traffic through Tor (requires running Tor daemon with ControlPort 9051)
        #[arg(long)]
        tor: bool,
    },
    /// Join an existing network using its room id or an invite code
    Join {
        /// The network public key (room id) or a one-time invite code
        network_key: String,
        /// Optional local alias for the network
        #[arg(long)]
        name: Option<String>,
        /// Your hostname within the network (e.g. "bob" → bob.gaming.ray). Random if not set
        #[arg(long)]
        hostname: Option<String>,
        /// Route traffic through Tor (requires running Tor daemon with ControlPort 9051)
        #[arg(long)]
        tor: bool,
    },
    /// Leave a network (remove from saved config)
    #[command(visible_alias = "rm")]
    Leave {
        /// Three-word network name
        name: String,
    },
    /// Destroy a network (coordinator only)
    Nuke {
        /// Three-word network name
        name: String,
        /// Force destroy even if other members exist
        #[arg(long)]
        force: bool,
    },
    /// Remove a member from a closed network (coordinator only)
    #[command(visible_alias = "boot")]
    Kick {
        /// Network name
        network: String,
        /// Member to remove: hostname, mesh IP, or short id
        peer: String,
    },
    /// Show status of all networks (active + saved)
    #[command(visible_aliases = ["st", "ls"])]
    Status,
    /// Run the daemon in the foreground (invoked by the system service)
    #[command(hide = true)]
    Daemon,
    /// Install the system service if needed and start it
    Up {
        /// Set your default hostname for future networks (e.g. "dario"). Used
        /// when create/join don't specify one; doesn't rename existing networks
        #[arg(long)]
        hostname: Option<String>,
    },
    /// Standby: take the data plane (TUN + Magic DNS) offline; stays connected to peers
    Down,
    /// Stop the system service (go fully offline). Requires root
    Stop,
    /// Start the installed system service. Requires root
    Start,
    /// Uninstall system service
    Uninstall,
    /// Install or refresh the system service and start it (requires root)
    Install,
    /// Restart the system service (requires root)
    Restart,
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// List peers awaiting approval on a closed network (coordinator only)
    Requests {
        /// Network name
        network: String,
    },
    /// Admit a peer waiting for approval (coordinator only)
    Accept {
        /// Network name
        network: String,
        /// Short id of the pending peer (from `torpedo requests`)
        id: String,
    },
    /// Reject a peer waiting for approval (coordinator only)
    Deny {
        /// Network name
        network: String,
        /// Short id of the pending peer (from `torpedo requests`)
        id: String,
    },
    /// Grant the network key to a member (coordinator only). The grantee becomes
    /// a co-coordinator: it can publish the signed blob and admit fresh joiners.
    /// Trusted-network multi-admin.
    Admin {
        /// Network name
        network: String,
        #[command(subcommand)]
        action: AdminAction,
    },
    /// View or change global daemon settings (relay, discovery-dns, subnet)
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Authorize a user to run torpedo without sudo (requires root)
    SetOperator {
        /// Username or numeric UID to grant operator access
        user: String,
    },
    /// Print the torpedo version
    #[command(visible_alias = "ver")]
    Version,
}

#[derive(Subcommand)]
pub(crate) enum AdminAction {
    /// Grant the network key to a member (coordinator only)
    Add {
        /// Short id of the member to promote (from `torpedo status`)
        identity: String,
    },
    /// List this network's key-holders (the local node + granted members)
    #[command(visible_alias = "ls")]
    List,
}

#[derive(Subcommand)]
pub(crate) enum ConfigAction {
    /// Show settings (all, or one key)
    #[command(visible_alias = "ls")]
    Get {
        /// relay, discovery-dns, or subnet (omit for all)
        key: Option<String>,
    },
    /// Set a key. Server keys take a comma list of presets (rayfish/n0)/URLs/IPs;
    /// `subnet` takes a single CIDR (e.g. 10.88.0.0/16). Applies on restart.
    Set {
        /// relay, discovery-dns, or subnet
        key: String,
        /// Server keys: comma list of presets/URLs/IPv4s. subnet: a CIDR.
        /// Empty resets to the default.
        value: String,
        /// Replace the defaults instead of augmenting them (server keys only)
        #[arg(long)]
        replace: bool,
    },
    /// Reset a key to its default (server keys -> iroh n0; subnet -> 10.88.0.0/16)
    #[command(visible_alias = "rm")]
    Unset {
        /// relay, discovery-dns, or subnet
        key: String,
    },
}

fn check_root() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("torpedo requires root privileges to create TUN devices. Run with sudo.");
        std::process::exit(1);
    }
}

/// Guard that must outlive the process: the file appender's `WorkerGuard`
/// (flushes buffered log lines).
#[derive(Default)]
struct LogGuard {
    _appender: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Build the tracing subscriber. The console layer (stdout) is always present;
/// the daemon additionally gets a rolling daily file layer under [`logdir::log_dir`]
/// so daemon activity is diagnosable after the fact.
/// The returned [`LogGuard`] must be kept alive for the lifetime of the process.
fn init_tracing(to_file: bool) -> LogGuard {
    use tracing_subscriber::prelude::*;

    // The global gate must be permissive enough for the most verbose layer (the
    // file), or events are dropped before any layer sees them. Default it to our
    // crate at `debug` (dependencies stay at `info` so iroh/quinn don't flood the
    // file), then keep the console quieter with a per-layer `info` filter below.
    // `RUST_LOG` overrides both, so an operator can still dial either up or down.
    let global_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tetron=debug"));
    let console_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Console layer — human text on stdout, held at `info` so CLI output and the
    // daemon console stay readable while the file keeps the `debug` detail.
    let console_layer = tracing_subscriber::fmt::layer().with_filter(console_filter);

    // File layer — daemon only, human text with ANSI stripped, rotated daily.
    let (file_layer, appender_guard) = if to_file {
        match std::fs::create_dir_all(logdir::log_dir()) {
            Ok(()) => {
                // Daily rotation; retain the 7 most recent files so logs older
                // than ~a week are pruned automatically (bounds disk usage).
                match tracing_appender::rolling::Builder::new()
                    .rotation(tracing_appender::rolling::Rotation::DAILY)
                    .filename_prefix("torpedo.log")
                    .max_log_files(7)
                    .build(logdir::log_dir())
                {
                    Ok(appender) => {
                        let (writer, guard) = tracing_appender::non_blocking(appender);
                        let layer = tracing_subscriber::fmt::layer()
                            .with_ansi(false)
                            .with_writer(writer);
                        (Some(layer), Some(guard))
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: cannot build rolling log appender: {e} (file logging disabled)"
                        );
                        (None, None)
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "warning: cannot create log directory {}: {e} (file logging disabled)",
                    logdir::log_dir().display()
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    let guard = LogGuard {
        _appender: appender_guard,
    };

    tracing_subscriber::registry()
        .with(global_filter)
        .with(console_layer)
        .with(file_layer)
        .init();
    guard
}

/// Install a fail-fast panic hook (daemon only). On any panic — including in a
/// spawned tokio task, which the runtime would otherwise swallow — it records the
/// crash (message, location, thread, backtrace) via `tracing::error!` (rolling file
/// log) and synchronously appends it to `panic.log` in the log
/// dir, then **aborts the process**.
///
/// Rationale: a panic is an invariant violation. For a VPN daemon, limping on with
/// a dead subsystem (e.g. a stalled forwarding loop) is worse than a clean restart —
/// and a live-but-broken process won't trip the service manager's restart. Aborting
/// lets systemd/launchd restart from known-good state; peers then reconnect. The
/// crash is captured durably in `panic.log`.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());

        tracing::error!(
            location = %location,
            thread = %thread,
            "panic: {message}\n{backtrace}"
        );
        // Durable, synchronous capture — survives even though abort() skips the
        // async log appender's flush.
        if let Err(e) = append_panic_log(&location, &thread, &message, &backtrace) {
            eprintln!("failed to write panic log: {e}");
        }

        // Print the standard panic message to stderr (journal), then fail fast so
        // the service manager restarts the daemon cleanly.
        default_hook(info);
        std::process::abort();
    }));
}

/// Append a panic record to `<log_dir>/panic.log`. Best-effort durability in case
/// the tracing pipeline itself is implicated in the crash.
fn append_panic_log(
    location: &str,
    thread: &str,
    message: &str,
    backtrace: &std::backtrace::Backtrace,
) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = logdir::log_dir();
    std::fs::create_dir_all(&dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("panic.log"))?;
    writeln!(f, "=== panic @ unix {ts} ===")?;
    writeln!(f, "thread:   {thread}")?;
    writeln!(f, "location: {location}")?;
    writeln!(f, "message:  {message}")?;
    writeln!(f, "backtrace:\n{backtrace}\n")?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.json {
        JSON_FLAG.store(true, atomic::Ordering::Relaxed);
    }
    // Keep the appender guard alive for the whole process so file logs flush.
    let _log_guard = init_tracing(matches!(cli.command, Command::Daemon));

    match cli.command {
        Command::Leave { name } => ipc_leave(&name).await,
        Command::Create {
            name,
            hostname,
            subnet,
            tor,
        } => ipc_create(GroupMode::Restricted, name, hostname, subnet, tor).await,
        Command::Join {
            network_key,
            name,
            hostname,
            tor,
        } => ipc_join(&network_key, name.as_deref(), hostname, tor).await,
        Command::Nuke { name, force } => ipc_nuke(&name, force).await,
        Command::Kick { network, peer } => ipc_kick(&network, &peer).await,
        Command::Status => ipc_status().await,
        Command::Daemon => {
            check_root();
            install_panic_hook();
            let token = shutdown::token();
            let stats = Arc::new(stats::ForwardMetrics::default());
            stats.spawn_logger(token.clone());
            daemon::run_daemon(token, stats).await
        }
        Command::Up { hostname } => cmd_up(hostname).await,
        Command::Down => ipc_down().await,
        Command::Stop => cmd_stop().await,
        Command::Start => cmd_start().await,
        Command::Uninstall => cmd_uninstall_service(),
        Command::Install => cmd_install().await,
        Command::Restart => cmd_restart().await,
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "torpedo", &mut std::io::stdout());
            Ok(())
        }
        Command::Requests { network } => ipc_requests(&network).await,
        Command::Accept { network, id } => ipc_accept_request(&network, &id).await,
        Command::Deny { network, id } => ipc_deny_request(&network, &id).await,
        Command::Admin { network, action } => ipc_admin(&network, action).await,
        Command::Config { action } => cmd_config(action, cli.json),
        Command::SetOperator { user } => cmd_set_operator(&user).await,
        Command::Version => {
            println!("torpedo {FULL_VERSION}");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Client-side commands (daemon optional)
// ---------------------------------------------------------------------------


/// `torpedo config get/set/unset`: view or change global daemon settings. Writes
/// `settings.toml` directly; relay/discovery/subnet all take effect on the next
/// daemon restart. On Linux the config tree is root-owned, so a write naturally
/// requires sudo.
fn cmd_config(action: Option<ConfigAction>, json: bool) -> Result<()> {
    match action.unwrap_or(ConfigAction::Get { key: None }) {
        ConfigAction::Get { key } => {
            // SUBNET-014: settings.toml is 0600 root:root (it holds
            // contact_secret_key), so a non-root caller cannot read it and
            // config::load() would silently return defaults — misreporting e.g.
            // `subnet` as <default>. Detect the unreadable file and hint to use
            // sudo instead of printing a wrong value.
            // The config dir (0750 root:torpedo) and settings.toml (0600
            // root:root) are inaccessible to a non-operator user, so opening it
            // fails with PermissionDenied — bail on that rather than let
            // config::load() fall back to a misleading default. (NotFound on a
            // fresh node is fine; load() handles it.)
            let settings = config::config_dir()?.join("settings.toml");
            if let Err(e) = std::fs::File::open(&settings)
                && e.kind() == std::io::ErrorKind::PermissionDenied
            {
                anyhow::bail!(
                    "config is root-only; re-run with sudo: sudo torpedo config get{}",
                    key.as_deref().map(|k| format!(" {k}")).unwrap_or_default()
                );
            }
            let cfg = config::load()?;
            let rows = config::config_get(&cfg, key.as_deref())?;
            if json {
                let map: serde_json::Map<String, serde_json::Value> = rows
                    .into_iter()
                    .map(|(k, v)| (k, serde_json::Value::String(v)))
                    .collect();
                print_json(&serde_json::Value::Object(map));
            } else {
                for (k, v) in rows {
                    println!("{k} = {v}");
                }
            }
        }
        ConfigAction::Set {
            key,
            value,
            replace,
        } => {
            let mut cfg = config::load()?;
            config::config_set(&mut cfg, &key, &value, replace)?;
            config::save_settings(&cfg)?;
            println!("Set {key}. Run 'sudo torpedo restart' for changes to take effect.");
        }
        ConfigAction::Unset { key } => {
            let mut cfg = config::load()?;
            config::config_set(&mut cfg, &key, "", false)?;
            config::save_settings(&cfg)?;
            println!("Reset {key} to default. Run 'sudo torpedo restart' for changes to take effect.");
        }
    }
    Ok(())
}

/// Resolve a username to its UID, falling back to parsing a numeric UID.
pub(crate) fn uid_for_user(user: &str) -> Option<u32> {
    use std::ffi::CString;
    let cname = CString::new(user).ok()?;
    let pw = unsafe { libc::getpwnam(cname.as_ptr()) };
    if !pw.is_null() {
        return Some(unsafe { (*pw).pw_uid });
    }
    user.parse::<u32>().ok()
}

/// `torpedo set-operator <user>`: authorize a local user to run mutating ray
/// commands without sudo (Tailscale's `--operator` model). The daemon enforces
/// that this call itself comes from root.
async fn cmd_set_operator(user: &str) -> Result<()> {
    let uid = uid_for_user(user)
        .ok_or_else(|| anyhow::anyhow!("unknown user '{user}' (pass a valid username or UID)"))?;
    let mut stream = ipc::connect()
        .await
        .context("torpedo daemon is not running; start it with: sudo torpedo up")?;
    ipc::send(&mut stream, ipc::IpcMessage::SetOperator { uid }).await?;
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

// ---------------------------------------------------------------------------
// IPC client commands (require daemon running)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_deleted_suffix_sanitizes_replaced_binary_path() {
        // After a manual upgrade unlinks the running binary, Linux reports
        // `/proc/self/exe` with a trailing " (deleted)". The service unit must
        // not inherit it, or the daemon crash-loops on `torpedo (deleted) daemon`.
        assert_eq!(
            strip_deleted_suffix("/usr/local/bin/torpedo (deleted)"),
            "/usr/local/bin/torpedo"
        );
        // A normal path is untouched.
        assert_eq!(
            strip_deleted_suffix("/usr/local/bin/torpedo"),
            "/usr/local/bin/torpedo"
        );
        // Only an exact trailing marker is stripped, not the substring mid-path.
        assert_eq!(
            strip_deleted_suffix("/opt/torpedo (deleted)/torpedo"),
            "/opt/torpedo (deleted)/torpedo"
        );
    }
}
