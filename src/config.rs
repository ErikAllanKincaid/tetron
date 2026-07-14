use std::fs::Permissions;
use std::net::Ipv4Addr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

use crate::membership::GroupMode;

/// Per-network transport preference. Defined in `ray-proto` (shared with GUI
/// frontends); re-exported here so existing `crate::config::TransportMode` paths work.
pub use tetron_proto::TransportMode;

#[allow(dead_code)]
mod secret_key_hex {
    use iroh::SecretKey;
    use serde::de::Error;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(key: &SecretKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(key.to_bytes()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SecretKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes: [u8; 32] = hex::decode(&s)
            .map_err(Error::custom)?
            .try_into()
            .map_err(|_| Error::custom("secret key must be 32 bytes"))?;
        Ok(SecretKey::from(bytes))
    }
}

mod option_secret_key_hex {
    use iroh::SecretKey;
    use serde::de::Error;
    use serde::{self, Deserializer, Serializer};

    pub fn serialize<S>(key: &Option<SecretKey>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match key {
            Some(k) => super::secret_key_hex::serialize(k, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SecretKey>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = serde::Deserialize::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let bytes: [u8; 32] = hex::decode(&s)
                    .map_err(Error::custom)?
                    .try_into()
                    .map_err(|_| Error::custom("secret key must be 32 bytes"))?;
                Ok(Some(SecretKey::from(bytes)))
            }
            None => Ok(None),
        }
    }
}

/// Info about a member in a saved network config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberEntry {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    #[serde(default)]
    pub is_coordinator: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// A pre-approved peer that hasn't connected yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedConfigEntry {
    pub identity: EndpointId,
    pub ip: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// A single saved network membership.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Human-friendly network alias (local only, not used for discovery).
    pub name: String,
    /// Membership mode: open or restricted.
    #[serde(default)]
    pub group_mode: GroupMode,
    /// Our assigned IP in this network (None if coordinator, Some if member).
    pub my_ip: Option<Ipv4Addr>,
    /// Our hostname in this network (persisted so it survives daemon restarts).
    /// Fixed at join (MINIMAL-014 removed rename); a member adopts the
    /// coordinator's authoritative (collision-resolved) name on reconverge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_hostname: Option<String>,
    /// Known members in this network.
    #[serde(default)]
    pub members: Vec<MemberEntry>,
    /// Pre-approved peers that haven't connected yet.
    #[serde(default)]
    pub approved: Vec<ApprovedConfigEntry>,
    #[serde(default, with = "option_secret_key_hex")]
    pub network_secret_key: Option<SecretKey>,
    #[serde(default)]
    pub network_public_key: Option<EndpointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<TransportMode>,
    /// Identities this coordinator has granted the per-network secret key to
    /// (`tetron admin add`). Local tracking only — the key is shared and not
    /// attributable, so this is the coordinator's record of grants, not a
    /// verifiable roster. Never published in the GroupBlob.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admins: Vec<EndpointId>,
    /// This is an auto-minted 2-peer "direct connection" network (`tetron connect`),
    /// not a user-created mesh. Tagged so `tetron status` can label it `[direct]`
    /// and suppress its (non-shareable) room id.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub direct: bool,
}

/// In-memory aggregate of the on-disk config. Reads assemble this from
/// `settings.toml` (globals) + one `networks/<name>.toml` per network; writes
/// are targeted (`save_settings` / `save_network` / `delete_network`) so a write
/// to one network can never clobber another. See the storage section below.
/// A global server override (relay / discovery-DNS / DNS-upstreams). `servers`
/// holds preset keywords (`rayfish`, `n0`) or literal URLs/IPs as the user typed
/// them; an empty list means unset (use the iroh n0 defaults). `replace` swaps
/// the defaults out instead of augmenting them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerOverride {
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default)]
    pub replace: bool,
}

impl ServerOverride {
    pub fn is_unset(&self) -> bool {
        self.servers.is_empty()
    }
}

/// Preset URL for the rayfish-operated iroh transport relay.
pub const RELAY_PRESET_RAYFISH: &str = "http://relay.iroh.rayfish.xyz:3340";
/// Preset URL for the rayfish-operated discovery-DNS / pkarr server.
pub const DISCOVERY_PRESET_RAYFISH: &str = "http://dns.iroh.rayfish.xyz:8080";

fn validate_http_url(s: &str) -> Result<()> {
    let u = url::Url::parse(s).with_context(|| format!("invalid URL: {s}"))?;
    anyhow::ensure!(
        matches!(u.scheme(), "http" | "https"),
        "URL must be http or https: {s}"
    );
    Ok(())
}

/// Resolve one relay/discovery entry: the `rayfish` keyword maps to `preset`,
/// anything else must be a valid http(s) URL (returned as-is).
fn resolve_url_entry(entry: &str, preset: &str) -> Result<String> {
    match entry {
        "rayfish" => Ok(preset.to_string()),
        other => {
            validate_http_url(other)?;
            Ok(other.to_string())
        }
    }
}

/// Resolve the relay override to concrete URL strings (presets expanded,
/// validated). Empty when unset.
pub fn relay_urls(o: &ServerOverride) -> Result<Vec<String>> {
    o.servers
        .iter()
        .map(|e| resolve_url_entry(e, RELAY_PRESET_RAYFISH))
        .collect()
}

/// Resolve the discovery-DNS override to concrete URL strings. Empty when unset.
pub fn discovery_urls(o: &ServerOverride) -> Result<Vec<String>> {
    o.servers
        .iter()
        .map(|e| resolve_url_entry(e, DISCOVERY_PRESET_RAYFISH))
        .collect()
}

/// Merge configured DNS upstreams with the system-captured ones. `replace`
/// drops the captured set; otherwise custom upstreams are tried first, then the
/// captured ones. Unset returns the captured set unchanged.
pub fn resolve_upstreams(o: &ServerOverride, captured: Vec<Ipv4Addr>) -> Vec<Ipv4Addr> {
    if o.servers.is_empty() {
        return captured;
    }
    let custom: Vec<Ipv4Addr> = o.servers.iter().filter_map(|s| s.parse().ok()).collect();
    if o.replace {
        custom
    } else {
        custom.into_iter().chain(captured).collect()
    }
}

/// Parse a comma list of entries (trimmed, empties dropped).
fn parse_entries(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Apply a `tetron config set`/`unset` to the in-memory config. An empty value or
/// the lone keyword `n0` resets the key to its default (iroh n0). Validates
/// every entry, so a bad URL/IP or unknown preset is rejected before persist.
pub fn config_set(cfg: &mut AppConfig, key: &str, value: &str, replace: bool) -> Result<()> {
    let entries = parse_entries(value);
    let reset = entries.is_empty() || entries == ["n0"];
    match key {
        "relay" => {
            if reset {
                cfg.relay = ServerOverride::default();
            } else {
                for e in &entries {
                    resolve_url_entry(e, RELAY_PRESET_RAYFISH)?;
                }
                cfg.relay = ServerOverride {
                    servers: entries,
                    replace,
                };
            }
        }
        "discovery-dns" => {
            if reset {
                cfg.discovery_dns = ServerOverride::default();
            } else {
                for e in &entries {
                    resolve_url_entry(e, DISCOVERY_PRESET_RAYFISH)?;
                }
                cfg.discovery_dns = ServerOverride {
                    servers: entries,
                    replace,
                };
            }
        }
        "subnet" => {
            // A single CIDR overlay subnet, not a URL list. Empty (or `n0`)
            // resets to the built-in default; `replace` is ignored here.
            if reset {
                cfg.subnet = None;
            } else {
                anyhow::ensure!(
                    entries.len() == 1,
                    "subnet takes a single CIDR, e.g. 10.88.0.0/16"
                );
                cfg.subnet = Some(crate::membership::parse_cidr(&entries[0])?);
            }
        }
        other => anyhow::bail!(
            "unknown config key: {other} (expected relay, discovery-dns, or subnet)"
        ),
    }
    Ok(())
}

fn render_override(o: &ServerOverride) -> String {
    if o.is_unset() {
        "<default>".to_string()
    } else {
        let mode = if o.replace { "replace" } else { "augment" };
        format!("{} ({mode})", o.servers.join(","))
    }
}

/// Render config settings as `(key, value)` rows for `tetron config get`. With a
/// key, returns just that one (error on unknown key); without, all three.
pub fn config_get(cfg: &AppConfig, key: Option<&str>) -> Result<Vec<(String, String)>> {
    let row = |k: &str| -> Result<(String, String)> {
        if k == "subnet" {
            let val = cfg
                .subnet
                .map(|(b, p)| format!("{b}/{p}"))
                .unwrap_or_else(|| "<default>".to_string());
            return Ok((k.to_string(), val));
        }
        let o = match k {
            "relay" => &cfg.relay,
            "discovery-dns" => &cfg.discovery_dns,
            other => anyhow::bail!(
                "unknown config key: {other} (expected relay, discovery-dns, or subnet)"
            ),
        };
        Ok((k.to_string(), render_override(o)))
    };
    match key {
        Some(k) => Ok(vec![row(k)?]),
        None => Ok(vec![row("relay")?, row("discovery-dns")?, row("subnet")?]),
    }
}

/// A closed-network join that was queued for coordinator approval and has not
/// yet been admitted. Persisted so the background retry survives a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingJoinEntry {
    /// The network public key (bare room id) we asked to join.
    pub network_key: String,
    /// The local display name to use once admitted, if the user gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Local UID authorized to control the daemon without root (Tailscale's
    /// `--operator` model). `None` means root-only for mutating commands.
    #[serde(default)]
    pub operator_uid: Option<u32>,
    /// Personal default hostname used when creating/joining a network without an
    /// explicit `--hostname`. Set via `tetron up --hostname <name>`. `None` falls
    /// back to a random generated name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_hostname: Option<String>,
    /// The node's operative overlay IPv4 subnet, cached locally as a CIDR string
    /// (e.g. "10.88.0.0/16"). The authoritative value lives in each network's
    /// signed `GroupBlob`; this is a read-through cache so the daemon can build
    /// its single TUN / identity in the right subnet at bootstrap, before any
    /// network is active. `None` means the default 10.88.0.0/16. Written by
    /// `create --subnet`, `join`, and `tetron config set subnet`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::membership::cidr_opt"
    )]
    pub subnet: Option<crate::membership::Subnet>,
    /// Custom iroh transport relay servers (NAT-traversal fallback).
    #[serde(default)]
    pub relay: ServerOverride,
    /// Custom iroh discovery-DNS / pkarr server (endpoint resolution + record
    /// publish). Also redirects the `dht.rs` pkarr client.
    #[serde(default)]
    pub discovery_dns: ServerOverride,
    #[serde(default)]
    pub networks: Vec<NetworkConfig>,
    /// Closed-network joins queued for coordinator approval, awaiting
    /// admission. See [`PendingJoinEntry`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_joins: Vec<PendingJoinEntry>,
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_joins: Vec<PendingJoinEntry>,
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
/// daemon's `~/.config/tetron` (root-only under `/var/root`).
pub fn config_dir() -> Result<PathBuf> {
    // An explicit `TORPEDO_CONFIG_DIR` override (renamed from the pre-fork
    // rayfish-prefixed name, RENAME-007, so it cannot collide with a genuine
    // rayfish process's own override on the same host) is honored only on Android
    // (`ray-mobile`'s `Node::new` points it at the app's `Context.getFilesDir()`)
    // and in `cfg(test)` (headless/test harnesses run against an isolated config
    // tree). Desktop/service production builds never check this var, so their
    // resolved path is byte-for-byte unchanged from before the override existed.
    #[cfg(any(target_os = "android", test))]
    if let Some(dir) = std::env::var_os("TORPEDO_CONFIG_DIR") {
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

/// Load the full config, assembling it from `settings.toml` + `networks/*.toml`.
/// Returns a default config if nothing is stored yet. Runs the legacy migration
/// on first call after an upgrade.
pub fn load() -> Result<AppConfig> {
    let dir = config_dir()?;
    migrate_legacy(&dir)?;
    load_in(&dir)
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
            pending_joins: Vec::new(),
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
        networks,
        pending_joins: settings.pending_joins,
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
        pending_joins: config.pending_joins.clone(),
    };
    let path = dir.join(SETTINGS_FILE);
    let contents = toml::to_string_pretty(&settings).context("serializing settings")?;
    write_atomic(&path, &contents, true)
}

/// Record a queued join so its background retry survives a daemon restart.
/// Idempotent: a second call with the same key updates the stored name but
/// does not duplicate the entry.
pub fn add_pending_join(entry: PendingJoinEntry) -> Result<()> {
    add_pending_join_in(&config_dir()?, entry)
}

fn add_pending_join_in(dir: &Path, entry: PendingJoinEntry) -> Result<()> {
    let mut cfg = load_in(dir)?;
    if let Some(existing) = cfg
        .pending_joins
        .iter_mut()
        .find(|e| e.network_key == entry.network_key)
    {
        existing.name = entry.name;
    } else {
        cfg.pending_joins.push(entry);
    }
    save_settings_in(dir, &cfg)
}

/// Drop a pending-join marker once the network is admitted (or abandoned).
pub fn remove_pending_join(network_key: &str) -> Result<()> {
    remove_pending_join_in(&config_dir()?, network_key)
}

fn remove_pending_join_in(dir: &Path, network_key: &str) -> Result<()> {
    let mut cfg = load_in(dir)?;
    let before = cfg.pending_joins.len();
    cfg.pending_joins.retain(|e| e.network_key != network_key);
    if cfg.pending_joins.len() != before {
        save_settings_in(dir, &cfg)?;
    }
    Ok(())
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

/// Add or update a network in the config. If a network with the same name
/// already exists, it is replaced.
pub fn upsert_network(config: &mut AppConfig, network: NetworkConfig) {
    if let Some(existing) = config.networks.iter_mut().find(|n| n.name == network.name) {
        *existing = network;
    } else {
        config.networks.push(network);
    }
}

/// Remove a network by name. Returns true if it was found and removed.
pub fn remove_network(config: &mut AppConfig, name: &str) -> bool {
    let before = config.networks.len();
    config.networks.retain(|n| n.name != name);
    config.networks.len() < before
}

/// Process-wide lock serializing tests that mutate `TORPEDO_CONFIG_DIR` (or any
/// other env var read by [`config_dir`]), since lib tests share one process and
/// run on parallel threads. Shared across test modules (`identity`, `daemon`)
/// so none of them observe a `TORPEDO_CONFIG_DIR` value set by a concurrent test.
#[cfg(test)]
pub(crate) static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::EndpointId;

    fn test_id(seed: u8) -> EndpointId {
        let mut key_bytes = [0u8; 32];
        key_bytes[0] = seed;
        SecretKey::from(key_bytes).public()
    }

    #[test]
    fn test_serialize_roundtrip() {
        let config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "gaming".to_string(),
                    group_mode: GroupMode::Open,
                    my_ip: Some(Ipv4Addr::new(100, 64, 10, 5)),
                    members: vec![
                        MemberEntry {
                            identity: test_id(2),
                            ip: Ipv4Addr::new(100, 64, 5, 3),
                            is_coordinator: true,
                            hostname: None,
                        },
                        MemberEntry {
                            identity: test_id(3),
                            ip: Ipv4Addr::new(100, 64, 10, 5),
                            is_coordinator: false,
                            hostname: None,
                        },
                    ],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                },
                NetworkConfig {
                    name: "work".to_string(),
                    group_mode: GroupMode::Restricted,
                    my_ip: None,
                    members: vec![],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                },
            ],
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.networks.len(), 2);
        assert_eq!(parsed.networks[0].name, "gaming");
        assert_eq!(parsed.networks[0].members.len(), 2);
        assert_eq!(parsed.networks[1].name, "work");
    }

    #[test]
    fn test_deserialize_empty() {
        let config: AppConfig = toml::from_str("").unwrap();
        assert!(config.networks.is_empty());
    }

    #[test]
    fn test_upsert_new() {
        let mut config = AppConfig::default();
        let net = NetworkConfig {
            name: "test".to_string(),
            group_mode: GroupMode::Open,
            my_ip: Some(Ipv4Addr::new(100, 64, 10, 5)),
            members: vec![],
            approved: vec![],
            network_secret_key: None,
            network_public_key: None,
            my_hostname: None,
            transport: None,
            admins: vec![],
            direct: false,
        };
        upsert_network(&mut config, net);
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "test");
        assert_eq!(config.networks[0].group_mode, GroupMode::Open);
    }

    #[test]
    fn test_upsert_replaces_existing() {
        let mut config = AppConfig {
            networks: vec![NetworkConfig {
                name: "test".to_string(),
                group_mode: GroupMode::Restricted,
                my_ip: None,
                members: vec![],
                approved: vec![],
                network_secret_key: None,
                network_public_key: None,
                my_hostname: None,
                transport: None,
                admins: vec![],
                direct: false,
            }],
            ..Default::default()
        };
        let updated = NetworkConfig {
            name: "test".to_string(),
            group_mode: GroupMode::Open,
            my_ip: Some(Ipv4Addr::new(100, 64, 10, 5)),
            members: vec![],
            approved: vec![],
            network_secret_key: None,
            network_public_key: None,
            my_hostname: None,
            transport: None,
            admins: vec![],
            direct: false,
        };
        upsert_network(&mut config, updated.clone());
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].group_mode, GroupMode::Open);
        assert_eq!(
            config.networks[0].my_ip,
            Some(Ipv4Addr::new(100, 64, 10, 5))
        );
    }

    #[test]
    fn test_remove_network() {
        let mut config = AppConfig {
            networks: vec![
                NetworkConfig {
                    name: "keep".to_string(),
                    group_mode: GroupMode::Restricted,
                    my_ip: None,
                    members: vec![],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                },
                NetworkConfig {
                    name: "remove-me".to_string(),
                    group_mode: GroupMode::Restricted,
                    my_ip: None,
                    members: vec![],
                    approved: vec![],
                    network_secret_key: None,
                    network_public_key: None,
                    my_hostname: None,
                    transport: None,
                    admins: vec![],
                    direct: false,
                },
            ],
            ..Default::default()
        };
        assert!(remove_network(&mut config, "remove-me"));
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "keep");
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut config = AppConfig::default();
        assert!(!remove_network(&mut config, "nope"));
    }

    #[test]
    fn test_serialize_with_approved() {
        let id1 = test_id(1);
        let id2 = test_id(2);
        let config = AppConfig {
            networks: vec![NetworkConfig {
                name: "gaming".to_string(),
                group_mode: GroupMode::Restricted,
                my_ip: Some(Ipv4Addr::new(100, 64, 10, 5)),
                members: vec![MemberEntry {
                    identity: id1,
                    ip: Ipv4Addr::new(100, 64, 5, 3),
                    is_coordinator: true,
                    hostname: None,
                }],
                approved: vec![ApprovedConfigEntry {
                    identity: id2,
                    ip: Ipv4Addr::new(100, 64, 12, 34),
                    hostname: None,
                }],
                network_secret_key: None,
                network_public_key: None,
                my_hostname: None,
                transport: None,
                admins: vec![],
                direct: false,
            }],
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.networks[0].approved.len(), 1);
        assert_eq!(parsed.networks[0].approved[0].identity, id2);
    }

    #[test]
    fn test_serialize_with_network_key() {
        let secret = SecretKey::generate();
        let public = secret.public();
        let config = AppConfig {
            networks: vec![NetworkConfig {
                name: "gaming".to_string(),
                group_mode: GroupMode::Restricted,
                my_ip: Some(Ipv4Addr::new(100, 64, 10, 5)),
                members: vec![],
                approved: vec![],
                network_secret_key: Some(secret.clone()),
                network_public_key: Some(public),
                my_hostname: None,
                transport: None,
                admins: vec![],
                direct: false,
            }],
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.networks[0].network_public_key, Some(public));
        assert!(parsed.networks[0].network_secret_key.is_some());
    }


    #[test]
    fn test_direct_flag_default_false() {
        let toml_str = r#"
[[networks]]
name = "dario-alice"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.networks[0].direct);
    }

    #[test]
    fn test_deserialize_minimal() {
        let toml_str = r#"
[[networks]]
name = "test"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].name, "test");
        assert_eq!(config.networks[0].group_mode, GroupMode::Restricted);
        assert!(config.networks[0].members.is_empty());
        assert!(config.networks[0].approved.is_empty());
        assert!(config.networks[0].network_secret_key.is_none());
        assert!(config.networks[0].network_public_key.is_none());
    }

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
        }
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

    #[test]
    fn relay_urls_expands_rayfish_preset() {
        let o = ServerOverride {
            servers: vec!["rayfish".into()],
            replace: false,
        };
        assert_eq!(
            relay_urls(&o).unwrap(),
            vec![RELAY_PRESET_RAYFISH.to_string()]
        );
        let d = ServerOverride {
            servers: vec!["rayfish".into()],
            replace: false,
        };
        assert_eq!(
            discovery_urls(&d).unwrap(),
            vec![DISCOVERY_PRESET_RAYFISH.to_string()]
        );
    }

    #[test]
    fn url_entry_rejects_bad() {
        assert!(
            relay_urls(&ServerOverride {
                servers: vec!["ftp://x".into()],
                replace: false
            })
            .is_err()
        );
        assert!(
            relay_urls(&ServerOverride {
                servers: vec!["not a url".into()],
                replace: false
            })
            .is_err()
        );
        // A real http URL passes through unchanged.
        let ok = ServerOverride {
            servers: vec!["http://r:1".into()],
            replace: false,
        };
        assert_eq!(relay_urls(&ok).unwrap(), vec!["http://r:1".to_string()]);
    }

    #[test]
    fn resolve_upstreams_augment_and_replace() {
        let captured = vec![Ipv4Addr::new(192, 168, 1, 1)];
        let one = Ipv4Addr::new(1, 1, 1, 1);

        // Unset: captured unchanged.
        assert_eq!(
            resolve_upstreams(&ServerOverride::default(), captured.clone()),
            captured
        );

        // Augment: custom first, then captured.
        let aug = ServerOverride {
            servers: vec!["1.1.1.1".into()],
            replace: false,
        };
        assert_eq!(
            resolve_upstreams(&aug, captured.clone()),
            vec![one, captured[0]]
        );

        // Replace: custom only.
        let rep = ServerOverride {
            servers: vec!["1.1.1.1".into()],
            replace: true,
        };
        assert_eq!(resolve_upstreams(&rep, captured.clone()), vec![one]);
    }

    #[test]
    fn config_set_unknown_key_errors() {
        let mut cfg = AppConfig::default();
        assert!(config_set(&mut cfg, "bogus", "rayfish", false).is_err());
        assert!(config_get(&cfg, Some("bogus")).is_err());
    }

    #[test]
    fn config_set_n0_resets() {
        let mut cfg = AppConfig::default();
        config_set(&mut cfg, "relay", "rayfish", true).unwrap();
        assert!(!cfg.relay.is_unset());
        config_set(&mut cfg, "relay", "n0", false).unwrap();
        assert!(cfg.relay.is_unset());
    }

    #[test]
    fn config_set_get_subnet() {
        use std::net::Ipv4Addr;
        let mut cfg = AppConfig::default();
        // Unset renders as <default>.
        assert_eq!(config_get(&cfg, Some("subnet")).unwrap()[0].1, "<default>");
        // Set a CIDR (stored raw, even distinct from default).
        config_set(&mut cfg, "subnet", "10.99.0.0/16", false).unwrap();
        assert_eq!(cfg.subnet, Some((Ipv4Addr::new(10, 99, 0, 0), 16)));
        assert_eq!(config_get(&cfg, Some("subnet")).unwrap()[0].1, "10.99.0.0/16");
        // Empty resets to default (None).
        config_set(&mut cfg, "subnet", "", false).unwrap();
        assert_eq!(cfg.subnet, None);
        // Garbage / bad prefix is rejected.
        assert!(config_set(&mut cfg, "subnet", "not-a-cidr", false).is_err());
        assert!(config_set(&mut cfg, "subnet", "10.0.0.0/33", false).is_err());
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

    #[test]
    fn pending_join_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Start from an empty settings file.
        save_settings_in(dir, &AppConfig::default()).unwrap();

        add_pending_join_in(
            dir,
            PendingJoinEntry {
                network_key: "abc123".to_string(),
                name: Some("homelab".to_string()),
            },
        )
        .unwrap();

        let loaded = load_in(dir).unwrap();
        assert_eq!(loaded.pending_joins.len(), 1);
        assert_eq!(loaded.pending_joins[0].network_key, "abc123");

        // Adding the same key again does not duplicate it.
        add_pending_join_in(
            dir,
            PendingJoinEntry {
                network_key: "abc123".to_string(),
                name: None,
            },
        )
        .unwrap();
        assert_eq!(load_in(dir).unwrap().pending_joins.len(), 1);

        remove_pending_join_in(dir, "abc123").unwrap();
        assert!(load_in(dir).unwrap().pending_joins.is_empty());
    }
}
