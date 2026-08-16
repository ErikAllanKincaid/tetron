//! Filesystem/persistence layer (MODULARIZE-003): config directory
//! resolution, atomic writes, permissions, migration, and load/save.
//! Re-exported from `crate::config` so every existing `crate::config::…`
//! path keeps compiling unchanged.

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::schema::{
    AppConfig, DropMonitorConfig, NetworkConfig, PathFlapConfig, RateLimitConfig,
    ReconnectColdConfig, ReconnectFrozenConfig, ReconnectLogConfig, ServerOverride,
};

// ---- Storage layout -------------------------------------------------------
//
// Config is sharded so a write to one network can never clobber another:
//
//   <config_dir>/settings.toml          globals (operator, default
//                                        hostname) — secret-bearing
//   <config_dir>/networks/<name>.toml   one NetworkConfig each — secret-bearing
//
// All writes go through `write_atomic` (temp file in the same dir + rename), so
// a concurrent reader never observes a torn file. This replaces the old single
// `networks.toml` whose non-atomic full-file rewrites raced under concurrent
// load-modify-save and silently dropped networks.
//
// Linux stores the tree under /etc/tetron owned root:tetron (see
// `config_dir`); secret-bearing files are 0600 root:root, dirs 0750
// root:tetron.

const LEGACY_FILE: &str = "networks.toml";
const SETTINGS_FILE: &str = "settings.toml";
const NETWORKS_SUBDIR: &str = "networks";

/// Globals persisted to `settings.toml` (everything in [`AppConfig`] except the
/// per-network entries, which live in their own files).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    operator_uid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_hostname: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::membership::cidr_opt"
    )]
    subnet: Option<crate::membership::Subnet>,
    #[serde(default)]
    relay: ServerOverride,
    #[serde(default)]
    discovery_dns: ServerOverride,
    #[serde(default)]
    ratelimit: RateLimitConfig,
    #[serde(default)]
    drop_monitor: DropMonitorConfig,
    #[serde(default)]
    path_flap: PathFlapConfig,
    #[serde(default)]
    reconnect_log: ReconnectLogConfig,
    #[serde(default)]
    reconnect_cold: ReconnectColdConfig,
    #[serde(default)]
    reconnect_frozen: ReconnectFrozenConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nuke_proposal_ttl: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    listen_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    poller_interval: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_retention: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invite_default_expiry: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selfcapture_mitigation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_level: Option<String>,
}

/// Look up the `tetron` group's gid (Linux), if the group exists.
#[cfg(target_os = "linux")]
fn tetron_gid() -> Option<u32> {
    use std::ffi::CString;
    let name = CString::new("tetron").ok()?;
    // SAFETY: getgrnam returns a pointer to a static struct; we copy gr_gid out
    // immediately before any further libc call could overwrite it.
    let grp = unsafe { libc::getgrnam(name.as_ptr()) };
    if grp.is_null() {
        None
    } else {
        Some(unsafe { (*grp).gr_gid })
    }
}

/// Best-effort `chown` to root, with group `tetron` for non-secret paths (or
/// root for secret ones). No-op off Linux. Silent on failure so the daemon
/// still starts if the group is missing.
#[cfg(target_os = "linux")]
fn set_owner(path: &Path, secret: bool) {
    let gid = if secret {
        Some(0)
    } else {
        tetron_gid().or(Some(0))
    };
    if let Err(e) = std::os::unix::fs::chown(path, Some(0), gid) {
        tracing::debug!(path = %path.display(), error = %e, "chown failed (non-fatal)");
    }
}

/// Create `dir` (and parents) with restrictive perms: 0750 root:tetron on
/// Linux. Idempotent.
fn ensure_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(target_os = "linux")]
    {
        let _ = std::fs::set_permissions(dir, Permissions::from_mode(0o750));
        set_owner(dir, false);
    }
    Ok(())
}

/// Base directory for all tetron config + state. Created if missing.
///
/// Linux: `/etc/tetron` (system service location, root:tetron). macOS: the
/// daemon's `~/Library/Application Support/tetron` (root-only under
/// `/var/root`, i.e. `/var/root/Library/Application Support/tetron`).
pub fn config_dir() -> Result<PathBuf> {
    // An explicit `TETRON_CONFIG_DIR` override (renamed from the torpedo-prefixed
    // name, RENAME-M02, so it cannot collide with a genuine upstream/prior-fork
    // process's own override on the same host). Originally honored only on
    // Android (a mobile embedder would point it at its app's
    // `Context.getFilesDir()`) and in `cfg(test)` (headless/test harnesses run
    // against an isolated config tree); widened (PORTABILITY-003) to every
    // build, since a real production install can have a genuine reason to
    // relocate this too (NixOS's non-FHS store layout is the motivating case --
    // see PLAN_CrossDistroPortability.md). An install that never sets the var
    // resolves the exact same path as before this existed -- purely additive.
    if let Some(dir) = std::env::var_os("TETRON_CONFIG_DIR") {
        let dir = PathBuf::from(dir);
        ensure_dir(&dir)?;
        return Ok(dir);
    }
    #[cfg(target_os = "linux")]
    let dir = PathBuf::from("/etc/tetron");
    // Android without the override falls back to a fixed app-private path so the
    // library still compiles/runs standalone.
    #[cfg(target_os = "android")]
    let dir = PathBuf::from("/data/local/tmp/tetron");
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let dir = dirs::config_dir()
        .context("could not determine config directory")?
        .join("tetron");
    ensure_dir(&dir)?;
    Ok(dir)
}

/// Reject a network name that can't be a safe single path component (defence in
/// depth — names are already validated as hostnames elsewhere).
fn validate_net_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        anyhow::bail!("invalid network name for config file: {name:?}");
    }
    Ok(())
}

/// Atomically write `bytes` to `path`: write a sibling temp file, set its
/// perms/owner, then rename over the target. The rename is atomic on POSIX, so
/// a concurrent reader sees either the old file or the new one — never a torn
/// one. `secret` selects 0600 root:root vs 0640 root:tetron.
///
/// Public so every tetron config writer (identity key, invite ledger, etc.)
/// shares the same atomic + restrictive-perms guarantees under the config tree.
pub fn write_file(path: &Path, bytes: &[u8], secret: bool) -> Result<()> {
    let dir = path.parent().context("config path has no parent")?;
    ensure_dir(dir)?;
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let tmp = dir.join(format!(".{fname}.tmp.{}", std::process::id()));
    {
        use std::io::Write;
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.sync_all().ok();
    }
    let mode = if secret { 0o600 } else { 0o640 };
    let _ = std::fs::set_permissions(&tmp, Permissions::from_mode(mode));
    #[cfg(target_os = "linux")]
    set_owner(&tmp, secret);
    let renamed = std::fs::rename(&tmp, path);
    if renamed.is_err() {
        // Clean up the temp file on a failed rename so we don't litter.
        let _ = std::fs::remove_file(&tmp);
    }
    renamed.with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

fn write_atomic(path: &Path, contents: &str, secret: bool) -> Result<()> {
    write_file(path, contents.as_bytes(), secret)
}

/// Apply restrictive perms/owner to an existing file under the config tree.
/// For append-mode files (e.g. the audit log) that aren't rewritten via
/// [`write_file`]. Best-effort.
pub fn restrict_perms(path: &Path, secret: bool) {
    let mode = if secret { 0o600 } else { 0o640 };
    let _ = std::fs::set_permissions(path, Permissions::from_mode(mode));
    #[cfg(target_os = "linux")]
    set_owner(path, secret);
}

/// Linux-only: relocate a pre-`/etc` config tree into `/etc/tetron` on first
/// start after the upgrade that moved the location. Earlier Linux builds stored
/// everything under the daemon's `~/.config/tetron` (i.e. `/root/.config`); this
/// moves `secret_key`, `networks.toml`, `invites/`, etc. over so
/// the node keeps its identity and networks. No-op on macOS (location unchanged)
/// and once `/etc/tetron` is populated. Must run before any config/identity read
/// (called at the top of `build_daemon`).
pub fn migrate_location() {
    #[cfg(target_os = "linux")]
    {
        let Ok(new) = config_dir() else { return };
        // Already populated → nothing to relocate.
        if new.join("secret_key").exists()
            || new.join(SETTINGS_FILE).exists()
            || new.join(LEGACY_FILE).exists()
            || new.join(NETWORKS_SUBDIR).is_dir()
        {
            return;
        }
        let Some(old) = dirs::config_dir().map(|d| d.join("tetron")) else {
            return;
        };
        if old == new || !old.is_dir() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&old) else {
            return;
        };
        let mut moved = 0;
        for e in entries.flatten() {
            let dest = new.join(e.file_name());
            // Same-filesystem rename is atomic; if it fails (e.g. EXDEV across
            // mounts) the entry is left in place and the daemon starts fresh —
            // logged so the operator can move it by hand.
            match std::fs::rename(e.path(), &dest) {
                Ok(()) => moved += 1,
                Err(err) => {
                    tracing::warn!(entry = ?e.path(), error = %err, "could not relocate config entry into /etc/tetron")
                }
            }
        }
        if moved > 0 {
            // Lock the relocated tree down: secrets keep old, possibly-loose perms
            // (older builds wrote the key without restricting it). Be conservative
            // — 0600 everything; later targeted writes relax non-secret files.
            if let Ok(entries) = std::fs::read_dir(&new) {
                for e in entries.flatten() {
                    if e.path().is_file() {
                        restrict_perms(&e.path(), true);
                    }
                }
            }
            tracing::info!(from = %old.display(), to = %new.display(), entries = moved, "relocated config tree to /etc/tetron");
        }
    }
}

/// One-time migration: split a legacy single `networks.toml` into the sharded
/// layout, keeping the original as `networks.toml.bak` (never deleted).
fn migrate_legacy(dir: &Path) -> Result<()> {
    let legacy = dir.join(LEGACY_FILE);
    if !legacy.exists() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(&legacy).context("reading legacy networks.toml")?;
    let old: AppConfig = toml::from_str(&contents).context("parsing legacy networks.toml")?;

    save_settings_in(dir, &old)?;
    for net in &old.networks {
        save_network_in(dir, net)?;
    }

    let bak = dir.join("networks.toml.bak");
    std::fs::rename(&legacy, &bak)
        .with_context(|| format!("renaming legacy config to {}", bak.display()))?;
    tracing::info!(backup = %bak.display(), networks = old.networks.len(), "migrated legacy config to per-network files");
    Ok(())
}

// --- CONFIG-CACHE-001: parsed-config cache, validated against the files ---
//
// `load()` sits in request-handling paths (`MeshManager::status()` calls it
// twice per request -- once directly, once per network inside
// `network_status()`), and `tetron-systray` polls `Status` every 8 seconds
// unconditionally. Re-reading and re-parsing the whole tree there worked out
// to roughly 1,800 file reads and 1,800 TOML parses per hour on a machine
// with a tray icon, to rebuild a value that had not changed.
//
// Validated against the filesystem rather than expired on a timer: the
// dominant caller polls on a fixed cadence, so any TTL short enough to bound
// staleness usefully would be missed by every poll, while a longer one would
// add a staleness window for nothing. Filesystem validation has no staleness
// window at all -- which matters because `tetron config set`, `join`, and
// `leave` write this tree from a *different process* than the daemon reading
// it, so in-process invalidation alone would be wrong.

/// Identity of the on-disk config tree: enough to tell "unchanged" from
/// "changed" without reading or parsing anything.
#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    /// `None` when `settings.toml` does not exist (a valid state -- `load_in`
    /// falls back to defaults -- and distinct from any existing file).
    settings: Option<(std::time::SystemTime, u64)>,
    /// Directory mtime, which an atomic rename into `networks/` updates. This
    /// is what catches a network being *added or removed*, and is why the
    /// validation path never needs its own `read_dir`.
    networks_dir: Option<std::time::SystemTime>,
    /// Per-file, catching a network edited *in place* (which leaves the
    /// directory mtime untouched).
    network_files: Vec<(std::ffi::OsString, std::time::SystemTime, u64)>,
}

struct CacheEntry {
    /// The cache is keyed by directory: `TETRON_CONFIG_DIR` can differ between
    /// calls (tests do exactly this), and a hit from another directory would
    /// be silently wrong.
    dir: PathBuf,
    fingerprint: Fingerprint,
    config: AppConfig,
}

static CONFIG_CACHE: std::sync::RwLock<Option<CacheEntry>> = std::sync::RwLock::new(None);

/// Counts full (cache-missing) loads so tests can assert the cache is
/// actually being used rather than merely returning correct values. Compiled
/// out entirely otherwise.
#[cfg(test)]
static FULL_LOADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn note_full_load() {
    #[cfg(test)]
    FULL_LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn stat_file(p: &Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn dir_mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Full fingerprint, enumerating `networks/`. Miss path only.
fn fingerprint_full(dir: &Path) -> Fingerprint {
    let ndir = dir.join(NETWORKS_SUBDIR);
    let mut network_files: Vec<(std::ffi::OsString, std::time::SystemTime, u64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&ndir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.extension().map(|x| x == "toml").unwrap_or(false) {
                continue;
            }
            if let Some((mt, len)) = stat_file(&p) {
                network_files.push((e.file_name(), mt, len));
            }
        }
    }
    network_files.sort();
    Fingerprint {
        settings: stat_file(&dir.join(SETTINGS_FILE)),
        networks_dir: dir_mtime(&ndir),
        network_files,
    }
}

/// Cheap validation: `stat`s only what the cached entry already names, with no
/// `read_dir`. A changed directory mtime means the *set* of networks may have
/// changed, which forces the full reload that re-enumerates it.
fn fingerprint_still_valid(dir: &Path, fp: &Fingerprint) -> bool {
    if stat_file(&dir.join(SETTINGS_FILE)) != fp.settings {
        return false;
    }
    let ndir = dir.join(NETWORKS_SUBDIR);
    if dir_mtime(&ndir) != fp.networks_dir {
        return false;
    }
    fp.network_files
        .iter()
        .all(|(name, mt, len)| stat_file(&ndir.join(name)) == Some((*mt, *len)))
}

/// Load the full config, assembling it from `settings.toml` + `networks/*.toml`.
/// Returns a default config if nothing is stored yet. Runs the legacy migration
/// on first call after an upgrade.
///
/// Cached (`CONFIG-CACHE-001`); the cached value is returned only while the
/// files it was built from are unchanged, so a write from any process is
/// picked up on the next call.
pub fn load() -> Result<AppConfig> {
    let dir = config_dir()?;
    load_cached_in(&dir, true)
}

/// `load()`'s body, with the directory injected and the legacy migration
/// optional so tests can exercise the cache against a tempdir.
fn load_cached_in(dir: &Path, migrate: bool) -> Result<AppConfig> {
    let hit = CONFIG_CACHE.read().ok().and_then(|guard| {
        guard
            .as_ref()
            .filter(|e| e.dir == dir && fingerprint_still_valid(dir, &e.fingerprint))
            .map(|e| e.config.clone())
    });
    if let Some(config) = hit {
        return Ok(config);
    }

    // Miss. `migrate_legacy` stays on this path only: it is idempotent and
    // effectively a once-at-startup step, so skipping it on a hit changes
    // nothing.
    if migrate {
        migrate_legacy(dir)?;
    }
    // Fingerprint *before* loading: if a write lands between the two, the
    // recorded fingerprint is the older one, so the next call sees a mismatch
    // and reloads. Taking it afterwards could record the newer stamp against
    // the older parse and cache a stale value indefinitely.
    let fingerprint = fingerprint_full(dir);
    let config = load_in(dir)?;
    note_full_load();

    if let Ok(mut guard) = CONFIG_CACHE.write() {
        *guard = Some(CacheEntry {
            dir: dir.to_path_buf(),
            fingerprint,
            config: config.clone(),
        });
    }
    Ok(config)
}

fn load_in(dir: &Path) -> Result<AppConfig> {
    let settings_path = dir.join(SETTINGS_FILE);
    let settings: Settings = if settings_path.exists() {
        let s = std::fs::read_to_string(&settings_path).context("reading settings.toml")?;
        toml::from_str(&s).context("parsing settings.toml")?
    } else {
        Settings {
            operator_uid: None,
            default_hostname: None,
            subnet: None,
            relay: ServerOverride::default(),
            discovery_dns: ServerOverride::default(),
            ratelimit: RateLimitConfig::default(),
            drop_monitor: DropMonitorConfig::default(),
            path_flap: PathFlapConfig::default(),
            reconnect_log: ReconnectLogConfig::default(),
            reconnect_cold: ReconnectColdConfig::default(),
            reconnect_frozen: ReconnectFrozenConfig::default(),
            nuke_proposal_ttl: None,
            listen_port: None,
            poller_interval: None,
            log_retention: None,
            invite_default_expiry: None,
            selfcapture_mitigation: None,
            log_level: None,
        }
    };

    let mut networks = Vec::new();
    let ndir = dir.join(NETWORKS_SUBDIR);
    if ndir.is_dir() {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&ndir)
            .with_context(|| format!("reading {}", ndir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "toml").unwrap_or(false))
            .collect();
        paths.sort();
        for p in paths {
            let s =
                std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
            // Atomic writes make a torn file unreachable, but be defensive: skip
            // an unparseable network rather than failing the whole load.
            match toml::from_str::<NetworkConfig>(&s) {
                Ok(nc) => networks.push(nc),
                Err(e) => {
                    tracing::warn!(path = %p.display(), error = %e, "skipping unreadable network config")
                }
            }
        }
    }

    Ok(AppConfig {
        operator_uid: settings.operator_uid,
        default_hostname: settings.default_hostname,
        subnet: settings.subnet,
        relay: settings.relay,
        discovery_dns: settings.discovery_dns,
        ratelimit: settings.ratelimit,
        drop_monitor: settings.drop_monitor,
        path_flap: settings.path_flap,
        reconnect_log: settings.reconnect_log,
        reconnect_cold: settings.reconnect_cold,
        reconnect_frozen: settings.reconnect_frozen,
        nuke_proposal_ttl: settings.nuke_proposal_ttl,
        listen_port: settings.listen_port,
        poller_interval: settings.poller_interval,
        log_retention: settings.log_retention,
        invite_default_expiry: settings.invite_default_expiry,
        selfcapture_mitigation: settings.selfcapture_mitigation,
        log_level: settings.log_level,
        networks,
    })
}

/// Persist the global settings (`settings.toml`) only. Does not touch networks.
pub fn save_settings(config: &AppConfig) -> Result<()> {
    save_settings_in(&config_dir()?, config)
}

/// The node's operative overlay subnet (cached in [`AppConfig::subnet`]), or the
/// default if unset/unreadable. Read at daemon bootstrap to build the TUN and
/// identity in the right range before any network is active.
pub fn node_subnet() -> crate::membership::Subnet {
    load()
        .ok()
        .and_then(|c| c.subnet)
        .unwrap_or_else(crate::membership::default_subnet)
}

/// Resolved `selfcapture-mitigation` value (SELFCAPTURE-ROUTE-001). Compiled
/// default is `true` (enabled).
pub fn selfcapture_mitigation_enabled() -> bool {
    load()
        .ok()
        .and_then(|c| c.selfcapture_mitigation)
        .unwrap_or(true)
}

/// Resolved `log-level` value (LOG-003) for the daemon's file log. Compiled
/// default is `"info"`. Read once at daemon startup by `init_tracing`;
/// `RUST_LOG` still wins over this if set.
pub fn log_level() -> String {
    load()
        .ok()
        .and_then(|c| c.log_level)
        .unwrap_or_else(|| "info".to_string())
}

/// Persist the node's operative overlay subnet (a local cache of the network's
/// authoritative `GroupBlob` value) so the daemon rebuilds its TUN/identity in
/// it at the next bootstrap. Stores `None` for the default subnet.
pub fn set_node_subnet(subnet: crate::membership::Subnet) -> Result<()> {
    let mut cfg = load()?;
    // Store the raw value (even if it equals the default) so an explicitly-chosen
    // subnet is distinguishable from "unset" (None) — SUBNET-010 relies on this to
    // reject a `create --subnet` that disagrees with the persisted node subnet.
    cfg.subnet = Some(subnet);
    save_settings(&cfg)
}

fn save_settings_in(dir: &Path, config: &AppConfig) -> Result<()> {
    let settings = Settings {
        operator_uid: config.operator_uid,
        default_hostname: config.default_hostname.clone(),
        subnet: config.subnet,
        relay: config.relay.clone(),
        discovery_dns: config.discovery_dns.clone(),
        ratelimit: config.ratelimit.clone(),
        drop_monitor: config.drop_monitor.clone(),
        path_flap: config.path_flap.clone(),
        reconnect_log: config.reconnect_log.clone(),
        reconnect_cold: config.reconnect_cold.clone(),
        reconnect_frozen: config.reconnect_frozen.clone(),
        nuke_proposal_ttl: config.nuke_proposal_ttl,
        listen_port: config.listen_port,
        poller_interval: config.poller_interval,
        log_retention: config.log_retention,
        invite_default_expiry: config.invite_default_expiry,
        selfcapture_mitigation: config.selfcapture_mitigation,
        log_level: config.log_level.clone(),
    };
    let path = dir.join(SETTINGS_FILE);
    let contents = toml::to_string_pretty(&settings).context("serializing settings")?;
    write_atomic(&path, &contents, true)
}

/// Persist a single network to `networks/<name>.toml`. Touches only that file,
/// so concurrent saves of distinct networks can never clobber one another.
pub fn save_network(net: &NetworkConfig) -> Result<()> {
    save_network_in(&config_dir()?, net)
}

fn save_network_in(dir: &Path, net: &NetworkConfig) -> Result<()> {
    validate_net_name(&net.name)?;
    let ndir = dir.join(NETWORKS_SUBDIR);
    let path = ndir.join(format!("{}.toml", net.name));
    let contents = toml::to_string_pretty(net).context("serializing network config")?;
    // Secret-bearing: holds the per-network coordinator secret key.
    write_atomic(&path, &contents, true)
}

/// Load a single network's config, if present.
pub fn load_network(name: &str) -> Result<Option<NetworkConfig>> {
    load_network_in(&config_dir()?, name)
}

fn load_network_in(dir: &Path, name: &str) -> Result<Option<NetworkConfig>> {
    validate_net_name(name)?;
    let path = dir.join(NETWORKS_SUBDIR).join(format!("{name}.toml"));
    if !path.exists() {
        return Ok(None);
    }
    let s =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(
        toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?,
    ))
}

/// Delete a single network's config file. Returns true if it existed.
pub fn delete_network(name: &str) -> Result<bool> {
    delete_network_in(&config_dir()?, name)
}

fn delete_network_in(dir: &Path, name: &str) -> Result<bool> {
    validate_net_name(name)?;
    let path = dir.join(NETWORKS_SUBDIR).join(format!("{name}.toml"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Process-wide lock serializing tests that mutate `TETRON_CONFIG_DIR` (or any
/// other env var read by [`config_dir`]), since lib tests share one process and
/// run on parallel threads. Shared across test modules (`identity`, `daemon`)
/// so none of them observe a `TETRON_CONFIG_DIR` value set by a concurrent test.
#[cfg(test)]
pub(crate) static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership::GroupMode;
    use iroh::SecretKey;

    fn net(name: &str) -> NetworkConfig {
        NetworkConfig {
            name: name.to_string(),
            group_mode: GroupMode::Restricted,
            my_ip: None,
            my_hostname: None,
            members: vec![],
            approved: vec![],
            network_secret_key: Some(SecretKey::generate()),
            network_public_key: None,
            transport: None,
            admins: vec![],
            direct: false,
            subnet: None,
            nuke_consensus_threshold: crate::membership::default_nuke_consensus_threshold(),
        }
    }

    // --- CONFIG-CACHE-001 ---------------------------------------------
    // These assert on FULL_LOADS, not just returned values: a cache that is
    // never consulted still returns correct results, so correctness alone
    // would pass whether or not the feature works.

    /// Serializes the cache tests against each other. They share one
    /// process-global cache and one global `FULL_LOADS` counter, so running
    /// two at once would make both flaky.
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn full_loads() -> usize {
        FULL_LOADS.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Drops the cache so a test starts from a known state regardless of what
    /// ran before it.
    fn clear_cache() {
        if let Ok(mut g) = CONFIG_CACHE.write() {
            *g = None;
        }
    }

    #[test]
    fn cached_load_skips_reparse_when_nothing_changed() {
        let _lk = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        save_network_in(dir, &net("homelab")).unwrap();
        clear_cache();

        let before = full_loads();
        let a = load_cached_in(dir, false).unwrap();
        assert_eq!(full_loads(), before + 1, "first call must do a full load");

        for _ in 0..5 {
            let b = load_cached_in(dir, false).unwrap();
            assert_eq!(b.networks.len(), a.networks.len());
        }
        assert_eq!(
            full_loads(),
            before + 1,
            "repeat calls with an unchanged tree must be served from cache"
        );
    }

    #[test]
    fn cached_load_sees_settings_change() {
        let _lk = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        save_settings_in(
            dir,
            &AppConfig {
                default_hostname: Some("first".into()),
                ..Default::default()
            },
        )
        .unwrap();
        clear_cache();

        let a = load_cached_in(dir, false).unwrap();
        assert_eq!(a.default_hostname.as_deref(), Some("first"));

        save_settings_in(
            dir,
            &AppConfig {
                default_hostname: Some("second-and-longer".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let b = load_cached_in(dir, false).unwrap();
        assert_eq!(
            b.default_hostname.as_deref(),
            Some("second-and-longer"),
            "a settings write from anywhere must invalidate the cache"
        );
    }

    #[test]
    fn cached_load_sees_network_added_and_removed() {
        let _lk = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        save_network_in(dir, &net("homelab")).unwrap();
        clear_cache();

        assert_eq!(load_cached_in(dir, false).unwrap().networks.len(), 1);

        save_network_in(dir, &net("genesis")).unwrap();
        assert_eq!(
            load_cached_in(dir, false).unwrap().networks.len(),
            2,
            "adding a network must invalidate the cache"
        );

        delete_network_in(dir, "genesis").unwrap();
        assert_eq!(
            load_cached_in(dir, false).unwrap().networks.len(),
            1,
            "removing a network must invalidate the cache"
        );
    }

    #[test]
    fn cached_load_sees_network_edited_in_place() {
        let _lk = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        save_network_in(dir, &net("homelab")).unwrap();
        clear_cache();
        assert_eq!(load_cached_in(dir, false).unwrap().networks.len(), 1);

        // Rewrite the file's bytes directly rather than via save_network_in,
        // which renames and so would move the directory mtime too. This is
        // the case only the per-file stat can catch.
        let p = dir.join(NETWORKS_SUBDIR).join("homelab.toml");
        let mut s = std::fs::read_to_string(&p).unwrap();
        s.push_str("\n# padding to change length and mtime\n");
        std::fs::write(&p, s).unwrap();

        let before = full_loads();
        let _ = load_cached_in(dir, false).unwrap();
        assert_eq!(
            full_loads(),
            before + 1,
            "an in-place edit leaves the directory mtime alone, so only the \
             per-file stat can catch it"
        );
        // Without this the assertion above would also hold with no cache at
        // all (every call is a full load then). This pins that the reload was
        // caused by the edit specifically, and that the cache re-arms after.
        let _ = load_cached_in(dir, false).unwrap();
        assert_eq!(
            full_loads(),
            before + 1,
            "the call after the edit-triggered reload must be a cache hit"
        );
    }

    #[test]
    fn cache_does_not_leak_across_directories() {
        let _lk = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        save_network_in(one.path(), &net("homelab")).unwrap();
        save_network_in(two.path(), &net("alpha")).unwrap();
        save_network_in(two.path(), &net("beta")).unwrap();
        clear_cache();

        assert_eq!(load_cached_in(one.path(), false).unwrap().networks.len(), 1);
        assert_eq!(
            load_cached_in(two.path(), false).unwrap().networks.len(),
            2,
            "a different config dir must never be served from another dir's entry"
        );
        assert_eq!(load_cached_in(one.path(), false).unwrap().networks.len(), 1);
    }

    #[test]
    fn per_network_roundtrip_and_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        save_network_in(dir, &net("homelab")).unwrap();
        save_network_in(dir, &net("genesis")).unwrap();
        save_settings_in(
            dir,
            &AppConfig {
                default_hostname: Some("dario".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let loaded = load_in(dir).unwrap();
        assert_eq!(loaded.networks.len(), 2);
        assert_eq!(loaded.default_hostname.as_deref(), Some("dario"));

        // Single-network load.
        assert!(load_network_in(dir, "homelab").unwrap().is_some());
        assert!(load_network_in(dir, "absent").unwrap().is_none());

        // Deleting one leaves the other untouched.
        assert!(delete_network_in(dir, "homelab").unwrap());
        assert!(!delete_network_in(dir, "homelab").unwrap());
        let after = load_in(dir).unwrap();
        assert_eq!(after.networks.len(), 1);
        assert_eq!(after.networks[0].name, "genesis");
    }

    #[test]
    fn settings_roundtrip_server_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // A fresh dir (no settings.toml) loads all three overrides as unset.
        let fresh = load_in(dir).unwrap();
        assert!(fresh.relay.is_unset());
        assert!(fresh.discovery_dns.is_unset());

        let cfg = AppConfig {
            relay: ServerOverride {
                servers: vec!["http://r:1".into()],
                replace: true,
            },
            ..Default::default()
        };
        save_settings_in(dir, &cfg).unwrap();

        let loaded = load_in(dir).unwrap();
        assert_eq!(loaded.relay, cfg.relay);
        assert!(loaded.discovery_dns.is_unset());
    }

    // Regression for the bug that prompted this change: concurrent saves of
    // distinct networks used to clobber one another through a single
    // non-atomic `networks.toml`. With one file per network they cannot.
    #[test]
    fn concurrent_saves_do_not_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        const N: usize = 24;

        std::thread::scope(|s| {
            for i in 0..N {
                let dir = dir.clone();
                s.spawn(move || {
                    save_network_in(&dir, &net(&format!("net-{i}"))).unwrap();
                });
            }
        });

        let loaded = load_in(&dir).unwrap();
        assert_eq!(
            loaded.networks.len(),
            N,
            "all concurrent saves must survive"
        );
    }

    #[test]
    fn migrate_legacy_splits_and_backs_up() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Write a legacy single-file config (the pre-shard format).
        let legacy = AppConfig {
            default_hostname: Some("dario".into()),
            networks: vec![net("homelab"), net("genesis")],
            ..Default::default()
        };
        std::fs::write(
            dir.join(LEGACY_FILE),
            toml::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        migrate_legacy(dir).unwrap();

        // Legacy file preserved as a backup, original gone.
        assert!(!dir.join(LEGACY_FILE).exists());
        assert!(dir.join("networks.toml.bak").exists());

        // Both networks + globals are now in the sharded layout.
        let loaded = load_in(dir).unwrap();
        assert_eq!(loaded.networks.len(), 2);
        assert_eq!(loaded.default_hostname.as_deref(), Some("dario"));

        // Idempotent: a second migrate (no legacy file) is a no-op.
        migrate_legacy(dir).unwrap();
        assert_eq!(load_in(dir).unwrap().networks.len(), 2);
    }

    #[test]
    fn rejects_unsafe_network_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(save_network_in(dir, &net("../escape")).is_err());
        assert!(load_network_in(dir, "a/b").is_err());
    }
}
